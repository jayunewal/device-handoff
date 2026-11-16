// SPDX-License-Identifier: Apache-2.0

//! Speaking to Apple's own Screen Sharing as a client of it
//! (`docs/adr/0020-screen-sharing-at-the-login-window.md`): the RFB protocol (RFC 6143) that
//! Screen Sharing's VNC server speaks at `127.0.0.1:5900`.
//!
//! This module is the protocol alone: the handshake, reading a picture, and sending a tap or a
//! key. It does not decide where a password for Screen Sharing's own sign-in comes from — the
//! record has the program draw a prompt of its own for that and use whatever is typed, not keep
//! one — nor does it turn what it reads into a picture for Baton's own session, or Baton's own
//! events into what it sends here. Those still need a `Windows` built around this and around
//! [`crate::console_session`], which ADR 0020's V1 and V2 answer first.
//!
//! Tested here against a server played in Rust, to the letter of RFC 6143, not against Screen
//! Sharing itself: V2 still checks which of its two sign-ins the login window actually asks for,
//! and only a real Mac answers that.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The two security types this client can satisfy on its own: RFC 6143's "None" and "VNC
/// Authentication". Apple's own account sign-in is a security type of its own that this client
/// does not speak yet (V2 has not yet said whether the login window even offers it).
const SECURITY_TYPE_NONE: u8 = 1;
const SECURITY_TYPE_VNC_AUTHENTICATION: u8 = 2;

/// The highest protocol version this client offers. RFC 6143 clients and servers agree on the
/// lower of the two they each support; `3.3` is the floor this speaks, for a server that old.
const HIGHEST_VERSION: (u16, u16) = (3, 8);

/// What went wrong reaching or signing in to Screen Sharing.
#[derive(Debug)]
pub(crate) enum ScreenSharingError {
    /// Reading or writing the connection failed.
    Io(std::io::Error),
    /// The server said something this client does not parse as RFB.
    Protocol(String),
    /// The server asked for a security type this client does not speak (Apple's own account
    /// sign-in is likely one, until this client speaks it too).
    UnsupportedSecurityType(Vec<u8>),
    /// The server asked for a password (VNC Authentication) and none was given.
    PasswordNeeded,
    /// The server refused the sign-in, with its own reason if it gave one (RFB 3.8 and VNC
    /// Authentication in every version give one; other cases may not).
    AuthenticationFailed(Option<String>),
}

impl std::fmt::Display for ScreenSharingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Protocol(message) => f.write_str(message),
            Self::UnsupportedSecurityType(types) => {
                write!(f, "the server offered only security types {types:?}")
            }
            Self::PasswordNeeded => f.write_str("the server asked for a password"),
            Self::AuthenticationFailed(Some(reason)) => f.write_str(reason),
            Self::AuthenticationFailed(None) => f.write_str("the sign-in was refused"),
        }
    }
}

impl std::error::Error for ScreenSharingError {}

impl From<std::io::Error> for ScreenSharingError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// How the server's picture is laid out in the bytes of each rectangle
/// (`ServerInit`/`SetPixelFormat`, RFC 6143 §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_color: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// 32 bits a pixel, 24 of them the picture, true colour, the shifts of an ordinary `RGBX`
    /// byte order. What [`set_pixel_format`] asks the server to use, so that a rectangle's bytes
    /// need no further work to read as colour.
    const RGBX_32: Self = Self {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_color: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 0,
        green_shift: 8,
        blue_shift: 16,
    };

    fn bytes_per_pixel(&self) -> usize {
        usize::from(self.bits_per_pixel) / 8
    }
}

/// What the server said of itself, once connected and signed in (`ServerInit`, RFC 6143 §7.3.2).
#[derive(Debug, Clone)]
pub(crate) struct ServerInfo {
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
    pub name: String,
}

/// One rectangle of a `FramebufferUpdate`, decoded from the `Raw` encoding (RFC 6143 §7.6.1,
/// §7.7.1): the only encoding this client asks for, so that reading one needs no codec of its
/// own, only the pixel format already agreed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// `width * height` pixels, each [`PixelFormat::bytes_per_pixel`] bytes, left to right then
    /// top to bottom.
    pub pixels: Vec<u8>,
}

/// Connects to Screen Sharing and signs in: the version and the security handshake, a password
/// for `VNC Authentication` if the server asks for it and `password` has one, and asking for the
/// pixel format [`PixelFormat::RGBX_32`] and the `Raw` encoding alone. Returns what the server
/// said of itself.
///
/// # Errors
///
/// See [`ScreenSharingError`].
pub(crate) async fn connect<S>(
    stream: &mut S,
    password: Option<&[u8]>,
) -> Result<ServerInfo, ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let server_version = read_version_line(stream).await?;
    let chosen = chosen_version(server_version);
    write_version_line(stream, chosen).await?;

    if chosen >= (3, 7) {
        negotiate_security_37(stream, password).await?;
    } else {
        negotiate_security_33(stream, password).await?;
    }

    write_client_init(stream, true).await?;
    read_server_init(stream).await
}

/// Asks for one incremental update of the whole picture (RFC 6143 §7.5.3).
pub(crate) async fn request_update<S>(
    stream: &mut S,
    info: &ServerInfo,
) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let mut message = Vec::with_capacity(10);
    message.push(3); // message-type: FramebufferUpdateRequest
    message.push(1); // incremental
    message.extend_from_slice(&0u16.to_be_bytes()); // x
    message.extend_from_slice(&0u16.to_be_bytes()); // y
    message.extend_from_slice(&info.width.to_be_bytes());
    message.extend_from_slice(&info.height.to_be_bytes());
    stream.write_all(&message).await?;
    Ok(())
}

/// Reads one `FramebufferUpdate`: every rectangle it holds, each decoded as `Raw` (the only
/// encoding this client asked for).
///
/// # Errors
///
/// [`ScreenSharingError::Protocol`] if a rectangle says it is encoded some other way.
pub(crate) async fn read_update<S>(
    stream: &mut S,
    pixel_format: &PixelFormat,
) -> Result<Vec<Rectangle>, ScreenSharingError>
where
    S: AsyncRead + Unpin,
{
    let message_type = read_u8(stream).await?;
    if message_type != 0 {
        return Err(ScreenSharingError::Protocol(format!(
            "expected a FramebufferUpdate (0), the server sent message type {message_type}"
        )));
    }
    let _padding = read_u8(stream).await?;
    let count = read_u16(stream).await?;
    let mut rectangles = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let x = read_u16(stream).await?;
        let y = read_u16(stream).await?;
        let width = read_u16(stream).await?;
        let height = read_u16(stream).await?;
        let encoding = read_i32(stream).await?;
        if encoding != 0 {
            return Err(ScreenSharingError::Protocol(format!(
                "asked for Raw alone, the server sent a rectangle encoded as {encoding}"
            )));
        }
        let byte_count = usize::from(width) * usize::from(height) * pixel_format.bytes_per_pixel();
        let mut pixels = vec![0_u8; byte_count];
        stream.read_exact(&mut pixels).await?;
        rectangles.push(Rectangle {
            x,
            y,
            width,
            height,
            pixels,
        });
    }
    Ok(rectangles)
}

/// A key going down or up, as an X11 keysym (RFC 6143 §7.5.4). Screen Sharing's own account
/// sign-in and the login window both read keys this way, the same as any other RFB server.
pub(crate) async fn send_key<S>(
    stream: &mut S,
    keysym: u32,
    down: bool,
) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let mut message = Vec::with_capacity(8);
    message.push(4); // message-type: KeyEvent
    message.push(u8::from(down));
    message.extend_from_slice(&0u16.to_be_bytes()); // padding
    message.extend_from_slice(&keysym.to_be_bytes());
    stream.write_all(&message).await?;
    Ok(())
}

/// A tap or a pointer move, with the buttons that are down (RFC 6143 §7.5.5).
pub(crate) async fn send_pointer<S>(
    stream: &mut S,
    x: u16,
    y: u16,
    button_mask: u8,
) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let mut message = Vec::with_capacity(6);
    message.push(5); // message-type: PointerEvent
    message.push(button_mask);
    message.extend_from_slice(&x.to_be_bytes());
    message.extend_from_slice(&y.to_be_bytes());
    stream.write_all(&message).await?;
    Ok(())
}

async fn negotiate_security_37<S>(
    stream: &mut S,
    password: Option<&[u8]>,
) -> Result<(), ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let count = read_u8(stream).await?;
    if count == 0 {
        let reason = read_reason_string(stream).await?;
        return Err(ScreenSharingError::AuthenticationFailed(Some(reason)));
    }
    let mut offered = vec![0_u8; usize::from(count)];
    stream.read_exact(&mut offered).await?;

    let chosen = if offered.contains(&SECURITY_TYPE_VNC_AUTHENTICATION) && password.is_some() {
        SECURITY_TYPE_VNC_AUTHENTICATION
    } else if offered.contains(&SECURITY_TYPE_NONE) {
        SECURITY_TYPE_NONE
    } else if offered.contains(&SECURITY_TYPE_VNC_AUTHENTICATION) {
        return Err(ScreenSharingError::PasswordNeeded);
    } else {
        return Err(ScreenSharingError::UnsupportedSecurityType(offered));
    };
    write_u8(stream, chosen).await?;
    authenticate(stream, chosen, password).await?;
    read_security_result(stream).await
}

async fn negotiate_security_33<S>(
    stream: &mut S,
    password: Option<&[u8]>,
) -> Result<(), ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let security_type = read_u32(stream).await?;
    match u8::try_from(security_type) {
        Ok(0) => {
            let reason = read_reason_string(stream).await?;
            Err(ScreenSharingError::AuthenticationFailed(Some(reason)))
        }
        Ok(SECURITY_TYPE_NONE) => Ok(()),
        Ok(SECURITY_TYPE_VNC_AUTHENTICATION) => {
            authenticate(stream, SECURITY_TYPE_VNC_AUTHENTICATION, password).await?;
            read_security_result(stream).await
        }
        _ => Err(ScreenSharingError::UnsupportedSecurityType(vec![])),
    }
}

async fn authenticate<S>(
    stream: &mut S,
    security_type: u8,
    password: Option<&[u8]>,
) -> Result<(), ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match security_type {
        SECURITY_TYPE_NONE => Ok(()),
        SECURITY_TYPE_VNC_AUTHENTICATION => {
            let Some(password) = password else {
                return Err(ScreenSharingError::PasswordNeeded);
            };
            let mut challenge = [0_u8; 16];
            stream.read_exact(&mut challenge).await?;
            let response = vnc_authentication_response(&challenge, password);
            stream.write_all(&response).await?;
            Ok(())
        }
        other => Err(ScreenSharingError::UnsupportedSecurityType(vec![other])),
    }
}

/// The 16-byte answer to a `VNC Authentication` challenge (RFC 6143 §7.2.2): the password, cut
/// to 8 bytes and padded with nulls if it is shorter, each of those bytes bit-reversed (a quirk
/// of the original VNC implementation that the RFC keeps), used as a DES key to encrypt the
/// 16-byte challenge as two independent 8-byte blocks.
fn vnc_authentication_response(challenge: &[u8; 16], password: &[u8]) -> [u8; 16] {
    use des::cipher::{BlockEncrypt, KeyInit};

    let mut key = [0_u8; 8];
    for (slot, byte) in key
        .iter_mut()
        .zip(password.iter().chain(std::iter::repeat(&0)))
    {
        *slot = byte.reverse_bits();
    }
    let cipher = des::Des::new(&key.into());

    let mut response = [0_u8; 16];
    for half in 0..2 {
        let mut block = des::cipher::generic_array::GenericArray::clone_from_slice(
            &challenge[half * 8..half * 8 + 8],
        );
        cipher.encrypt_block(&mut block);
        response[half * 8..half * 8 + 8].copy_from_slice(&block);
    }
    response
}

async fn read_security_result<S>(stream: &mut S) -> Result<(), ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = read_u32(stream).await?;
    if result == 0 {
        return Ok(());
    }
    // A reason string follows only from RFB 3.8 onward; a server that negotiated an older
    // version down may close the connection instead of sending one, which surfaces as an I/O
    // error from the next read rather than from here.
    let reason = read_reason_string(stream).await.ok();
    Err(ScreenSharingError::AuthenticationFailed(reason))
}

async fn read_reason_string<S>(stream: &mut S) -> Result<String, ScreenSharingError>
where
    S: AsyncRead + Unpin,
{
    let length = read_u32(stream).await?;
    let mut bytes = vec![0_u8; length as usize];
    stream.read_exact(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn write_client_init<S>(stream: &mut S, shared: bool) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(&[u8::from(shared)]).await?;
    Ok(())
}

async fn read_server_init<S>(stream: &mut S) -> Result<ServerInfo, ScreenSharingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let width = read_u16(stream).await?;
    let height = read_u16(stream).await?;

    let mut format_bytes = [0_u8; 16];
    stream.read_exact(&mut format_bytes).await?;
    let server_format = PixelFormat {
        bits_per_pixel: format_bytes[0],
        depth: format_bytes[1],
        big_endian: format_bytes[2] != 0,
        true_color: format_bytes[3] != 0,
        red_max: u16::from_be_bytes([format_bytes[4], format_bytes[5]]),
        green_max: u16::from_be_bytes([format_bytes[6], format_bytes[7]]),
        blue_max: u16::from_be_bytes([format_bytes[8], format_bytes[9]]),
        red_shift: format_bytes[10],
        green_shift: format_bytes[11],
        blue_shift: format_bytes[12],
    };
    let _ = server_format; // Superseded at once by set_pixel_format, below.

    let name_length = read_u32(stream).await?;
    let mut name_bytes = vec![0_u8; name_length as usize];
    stream.read_exact(&mut name_bytes).await?;
    let name = String::from_utf8_lossy(&name_bytes).into_owned();

    set_pixel_format(stream, &PixelFormat::RGBX_32).await?;
    set_encodings(stream, &[0]).await?; // Raw alone.

    Ok(ServerInfo {
        width,
        height,
        pixel_format: PixelFormat::RGBX_32,
        name,
    })
}

async fn set_pixel_format<S>(stream: &mut S, format: &PixelFormat) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let mut message = vec![0_u8; 20];
    message[0] = 0; // message-type: SetPixelFormat
    message[4] = format.bits_per_pixel;
    message[5] = format.depth;
    message[6] = u8::from(format.big_endian);
    message[7] = u8::from(format.true_color);
    message[8..10].copy_from_slice(&format.red_max.to_be_bytes());
    message[10..12].copy_from_slice(&format.green_max.to_be_bytes());
    message[12..14].copy_from_slice(&format.blue_max.to_be_bytes());
    message[14] = format.red_shift;
    message[15] = format.green_shift;
    message[16] = format.blue_shift;
    stream.write_all(&message).await?;
    Ok(())
}

async fn set_encodings<S>(stream: &mut S, encodings: &[i32]) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let mut message = Vec::with_capacity(4 + encodings.len() * 4);
    message.push(2); // message-type: SetEncodings
    message.push(0); // padding
    let count = u16::try_from(encodings.len())
        .map_err(|_| ScreenSharingError::Protocol("too many encodings to ask for".to_owned()))?;
    message.extend_from_slice(&count.to_be_bytes());
    for encoding in encodings {
        message.extend_from_slice(&encoding.to_be_bytes());
    }
    stream.write_all(&message).await?;
    Ok(())
}

/// The higher of the two protocol versions this client and the server each offer, down to the
/// lowest this client still speaks.
fn chosen_version(server: (u16, u16)) -> (u16, u16) {
    if server >= HIGHEST_VERSION {
        HIGHEST_VERSION
    } else if server < (3, 3) {
        (3, 3)
    } else {
        server
    }
}

async fn read_version_line<S>(stream: &mut S) -> Result<(u16, u16), ScreenSharingError>
where
    S: AsyncRead + Unpin,
{
    let mut line = [0_u8; 12];
    stream.read_exact(&mut line).await?;
    parse_version_line(&line)
}

fn parse_version_line(line: &[u8; 12]) -> Result<(u16, u16), ScreenSharingError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| ScreenSharingError::Protocol("the version line is not UTF-8".to_owned()))?;
    let digits = text
        .strip_prefix("RFB ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .ok_or_else(|| {
            ScreenSharingError::Protocol(format!("{text:?} does not open like an RFB version"))
        })?;
    let (major, minor) = digits.split_once('.').ok_or_else(|| {
        ScreenSharingError::Protocol(format!("{text:?} does not open like an RFB version"))
    })?;
    let major = major.parse().map_err(|_| {
        ScreenSharingError::Protocol(format!("{text:?} does not open like an RFB version"))
    })?;
    let minor = minor.parse().map_err(|_| {
        ScreenSharingError::Protocol(format!("{text:?} does not open like an RFB version"))
    })?;
    Ok((major, minor))
}

async fn write_version_line<S>(
    stream: &mut S,
    version: (u16, u16),
) -> Result<(), ScreenSharingError>
where
    S: AsyncWrite + Unpin,
{
    let line = format!("RFB {:03}.{:03}\n", version.0, version.1);
    stream.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn read_u8<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<u8> {
    let mut byte = [0_u8; 1];
    stream.read_exact(&mut byte).await?;
    Ok(byte[0])
}

async fn read_u16<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<u16> {
    let mut bytes = [0_u8; 2];
    stream.read_exact(&mut bytes).await?;
    Ok(u16::from_be_bytes(bytes))
}

async fn read_i32<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<i32> {
    let mut bytes = [0_u8; 4];
    stream.read_exact(&mut bytes).await?;
    Ok(i32::from_be_bytes(bytes))
}

async fn read_u32<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<u32> {
    let mut bytes = [0_u8; 4];
    stream.read_exact(&mut bytes).await?;
    Ok(u32::from_be_bytes(bytes))
}

async fn write_u8<S: AsyncWrite + Unpin>(stream: &mut S, value: u8) -> std::io::Result<()> {
    stream.write_all(&[value]).await
}

#[cfg(test)]
mod tests {
    use tokio::io::DuplexStream;

    use super::*;

    /// A connected pair: `client` is given to the code under test, `server` is driven by the
    /// test itself, playing the part of Screen Sharing to the letter of RFC 6143.
    fn pipe() -> (DuplexStream, DuplexStream) {
        tokio::io::duplex(4096)
    }

    #[tokio::test]
    async fn connects_with_no_security_at_all() {
        let (mut client, mut server) = pipe();
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();
            assert_eq!(&client_version, b"RFB 003.008\n");

            server.write_all(&[1, SECURITY_TYPE_NONE]).await.unwrap();
            let mut chosen = [0_u8; 1];
            server.read_exact(&mut chosen).await.unwrap();
            assert_eq!(chosen[0], SECURITY_TYPE_NONE);
            server.write_all(&0u32.to_be_bytes()).await.unwrap(); // SecurityResult: OK

            let mut client_init = [0_u8; 1];
            server.read_exact(&mut client_init).await.unwrap();

            server.write_all(&800u16.to_be_bytes()).await.unwrap(); // width
            server.write_all(&600u16.to_be_bytes()).await.unwrap(); // height
            server.write_all(&[0_u8; 16]).await.unwrap(); // pixel format, ignored by the client
            let name = b"the login window";
            server
                .write_all(&u32::try_from(name.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            server.write_all(name).await.unwrap();

            // SetPixelFormat (20 bytes) and SetEncodings (4 + 4 bytes) follow at once.
            let mut set_pixel_format = [0_u8; 20];
            server.read_exact(&mut set_pixel_format).await.unwrap();
            assert_eq!(set_pixel_format[0], 0);
            let mut set_encodings = [0_u8; 8];
            server.read_exact(&mut set_encodings).await.unwrap();
            assert_eq!(set_encodings[0], 2);
        });

        let info = connect(&mut client, None).await.unwrap();
        assert_eq!((info.width, info.height), (800, 600));
        assert_eq!(info.name, "the login window");
        assert_eq!(info.pixel_format, PixelFormat::RGBX_32);

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn signs_in_with_a_password_when_vnc_authentication_is_asked_for() {
        let (mut client, mut server) = pipe();
        let challenge = [7_u8; 16];
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();

            server
                .write_all(&[1, SECURITY_TYPE_VNC_AUTHENTICATION])
                .await
                .unwrap();
            let mut chosen = [0_u8; 1];
            server.read_exact(&mut chosen).await.unwrap();
            assert_eq!(chosen[0], SECURITY_TYPE_VNC_AUTHENTICATION);

            server.write_all(&challenge).await.unwrap();
            let mut response = [0_u8; 16];
            server.read_exact(&mut response).await.unwrap();
            assert_eq!(response, vnc_authentication_response(&challenge, b"sesame"));

            server.write_all(&0u32.to_be_bytes()).await.unwrap(); // SecurityResult: OK
            let mut client_init = [0_u8; 1];
            server.read_exact(&mut client_init).await.unwrap();
            server.write_all(&1u16.to_be_bytes()).await.unwrap();
            server.write_all(&1u16.to_be_bytes()).await.unwrap();
            server.write_all(&[0_u8; 16]).await.unwrap();
            server.write_all(&0u32.to_be_bytes()).await.unwrap(); // empty name
            let mut set_pixel_format = [0_u8; 20];
            server.read_exact(&mut set_pixel_format).await.unwrap();
            let mut set_encodings = [0_u8; 8];
            server.read_exact(&mut set_encodings).await.unwrap();
        });

        connect(&mut client, Some(b"sesame")).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_wrong_password_is_reported_as_the_server_explains_it() {
        let (mut client, mut server) = pipe();
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();
            server
                .write_all(&[1, SECURITY_TYPE_VNC_AUTHENTICATION])
                .await
                .unwrap();
            let mut chosen = [0_u8; 1];
            server.read_exact(&mut chosen).await.unwrap();
            server.write_all(&[9_u8; 16]).await.unwrap(); // challenge
            let mut response = [0_u8; 16];
            server.read_exact(&mut response).await.unwrap();

            server.write_all(&1u32.to_be_bytes()).await.unwrap(); // SecurityResult: failed
            let reason = b"wrong password";
            server
                .write_all(&u32::try_from(reason.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            server.write_all(reason).await.unwrap();
        });

        let error = connect(&mut client, Some(b"nope")).await.unwrap_err();
        assert_eq!(error.to_string(), "wrong password");
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn asks_for_a_password_it_was_not_given() {
        let (mut client, mut server) = pipe();
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();
            // Only VNC Authentication is offered, and the client was given no password.
            server
                .write_all(&[1, SECURITY_TYPE_VNC_AUTHENTICATION])
                .await
                .unwrap();
        });

        let error = connect(&mut client, None).await.unwrap_err();
        assert!(
            matches!(error, ScreenSharingError::PasswordNeeded),
            "{error:?}"
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_security_type_this_client_does_not_speak_is_named_and_not_guessed_at() {
        let (mut client, mut server) = pipe();
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();
            // Type 30 is Apple's own account sign-in, which this client does not speak yet.
            server.write_all(&[1, 30]).await.unwrap();
        });

        let error = connect(&mut client, None).await.unwrap_err();
        assert!(
            matches!(&error, ScreenSharingError::UnsupportedSecurityType(types) if types == &[30]),
            "{error:?}"
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn an_older_server_is_met_at_its_own_version_and_names_its_security_type_directly() {
        let (mut client, mut server) = pipe();
        let server_task = tokio::spawn(async move {
            server.write_all(b"RFB 003.003\n").await.unwrap();
            let mut client_version = [0_u8; 12];
            server.read_exact(&mut client_version).await.unwrap();
            assert_eq!(&client_version, b"RFB 003.003\n");

            server
                .write_all(&u32::from(SECURITY_TYPE_NONE).to_be_bytes())
                .await
                .unwrap();
            let mut client_init = [0_u8; 1];
            server.read_exact(&mut client_init).await.unwrap();
            server.write_all(&1u16.to_be_bytes()).await.unwrap();
            server.write_all(&1u16.to_be_bytes()).await.unwrap();
            server.write_all(&[0_u8; 16]).await.unwrap();
            server.write_all(&0u32.to_be_bytes()).await.unwrap();
            let mut set_pixel_format = [0_u8; 20];
            server.read_exact(&mut set_pixel_format).await.unwrap();
            let mut set_encodings = [0_u8; 8];
            server.read_exact(&mut set_encodings).await.unwrap();
        });

        connect(&mut client, None).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_raw_rectangle_and_refuses_one_encoded_another_way() {
        let (mut client, mut server) = pipe();
        let pixel_format = PixelFormat::RGBX_32;

        // A rectangle of width 2 and height 1, so its byte count is 2 pixels' worth.
        let pixel_bytes = 2 * pixel_format.bytes_per_pixel();
        let good = tokio::spawn(async move {
            let mut message = Vec::new();
            message.push(0_u8); // FramebufferUpdate
            message.push(0); // padding
            message.extend_from_slice(&1u16.to_be_bytes()); // one rectangle
            message.extend_from_slice(&1u16.to_be_bytes()); // x
            message.extend_from_slice(&2u16.to_be_bytes()); // y
            message.extend_from_slice(&2u16.to_be_bytes()); // width
            message.extend_from_slice(&1u16.to_be_bytes()); // height
            message.extend_from_slice(&0i32.to_be_bytes()); // Raw
            let pixels = vec![9_u8; pixel_bytes];
            message.extend_from_slice(&pixels);
            server.write_all(&message).await.unwrap();
            server.shutdown().await.unwrap();
        });
        let rectangles = read_update(&mut client, &pixel_format).await.unwrap();
        assert_eq!(rectangles.len(), 1);
        assert_eq!(rectangles[0].x, 1);
        assert_eq!(rectangles[0].y, 2);
        assert_eq!(rectangles[0].width, 2);
        assert_eq!(rectangles[0].height, 1);
        assert_eq!(rectangles[0].pixels, vec![9_u8; pixel_bytes]);
        good.await.unwrap();

        let (mut client, mut server) = pipe();
        let bad = tokio::spawn(async move {
            let mut message = Vec::new();
            message.push(0_u8);
            message.push(0);
            message.extend_from_slice(&1u16.to_be_bytes());
            message.extend_from_slice(&0u16.to_be_bytes());
            message.extend_from_slice(&0u16.to_be_bytes());
            message.extend_from_slice(&1u16.to_be_bytes());
            message.extend_from_slice(&1u16.to_be_bytes());
            message.extend_from_slice(&5i32.to_be_bytes()); // Hextile, not asked for
            server.write_all(&message).await.unwrap();
            server.shutdown().await.unwrap();
        });
        let error = read_update(&mut client, &pixel_format).await.unwrap_err();
        assert!(
            matches!(error, ScreenSharingError::Protocol(_)),
            "{error:?}"
        );
        bad.await.unwrap();
    }

    #[tokio::test]
    async fn sends_a_key_and_a_tap_as_the_wire_says_to() {
        let (mut client, mut server) = pipe();
        send_key(&mut client, 0xff0d, true).await.unwrap(); // Return, down
        let mut key_message = [0_u8; 8];
        server.read_exact(&mut key_message).await.unwrap();
        assert_eq!(key_message, [4, 1, 0, 0, 0, 0, 0xff, 0x0d]);

        send_pointer(&mut client, 10, 20, 0b001).await.unwrap();
        let mut pointer_message = [0_u8; 6];
        server.read_exact(&mut pointer_message).await.unwrap();
        assert_eq!(pointer_message, [5, 0b001, 0, 10, 0, 20]);
    }

    #[test]
    fn the_vnc_authentication_key_reverses_the_bits_of_each_byte_and_pads_with_nulls() {
        // A password byte of 0b0000_0001 becomes a key byte of 0b1000_0000, and a password
        // shorter than 8 bytes is padded with nulls, not repeated or cut from the wrong end.
        let challenge = [0_u8; 16];
        let short = vnc_authentication_response(&challenge, &[0b0000_0001]);
        let padded = vnc_authentication_response(&challenge, &[0b0000_0001, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(short, padded);

        let a = vnc_authentication_response(&challenge, b"a");
        let b = vnc_authentication_response(&challenge, b"b");
        assert_ne!(
            a, b,
            "different keys must encrypt the same challenge differently"
        );
    }
}
