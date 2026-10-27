// SPDX-License-Identifier: Apache-2.0

// What the screens need from Baton, as two small interfaces. The app is written against these,
// so that widget tests can run it on any computer with a fake behind them, and so that the one
// place that touches the Rust library is rust_backend.dart.

import 'rust/api/model.dart';

export 'rust/api/error.dart' show BatonError, ErrorKind;
export 'rust/api/model.dart'
    show
        Device,
        Event,
        Event_DevicesChanged,
        Event_OfferEnded,
        Event_PairingRequestEnded,
        Event_PairingRequested,
        Event_Received,
        Event_Resync,
        Event_TransferEnded,
        Event_TransferRequestEnded,
        Event_TransferRequested,
        Event_WindowEnded,
        Event_WindowNotice,
        Event_WindowPicture,
        Event_WindowRequestEnded,
        Event_WindowRequested,
        Event_WindowSound,
        Event_WindowStarted,
        FileProgress,
        FilesSent,
        JoinProgress,
        JoinProgress_Code,
        JoinProgress_Failed,
        JoinProgress_Paired,
        Me,
        Network,
        Offer,
        OfferOutcome,
        OfferOutcome_Abandoned,
        OfferOutcome_Cancelled,
        OfferOutcome_Declined,
        OfferOutcome_Expired,
        OfferOutcome_Failed,
        OfferOutcome_Paired,
        PairingRequest,
        Platform,
        PreviewName,
        Reach,
        Received,
        ReceivedItem,
        ReceivedItem_Link,
        ReceivedItem_Text,
        Receiving,
        Receiving_Off,
        Receiving_On,
        Receiving_Unusable,
        Route,
        ScreenStarted,
        ScreenStarted_NotStarted,
        ScreenStarted_Started,
        SendStage,
        StartAccess,
        TransferEnd,
        TransferRequest,
        TransferResult,
        TransferResult_Dropped,
        TransferResult_Received,
        WindowAbilities,
        WindowButton,
        WindowEnd,
        WindowEnd_Failed,
        WindowEnd_Lost,
        WindowEnd_OtherDevice,
        WindowEnd_ThisDevice,
        WindowInput,
        WindowInput_Key,
        WindowInput_Pointer,
        WindowModifiers,
        WindowNoticeReason,
        WindowPointer,
        WindowPointer_Down,
        WindowPointer_Move,
        WindowPointer_Scroll,
        WindowPointer_Up,
        WindowRequest,
        WindowRole,
        WindowSession,
        WindowShared,
        WindowShared_Declined,
        WindowShared_NothingChosen,
        WindowShared_Started,
        WindowSound;

/// Opens the folder where Baton keeps its files and starts Baton in it.
abstract interface class Backend {
  /// The name saved for this device, or `null` if the person has not chosen one yet.
  Future<String?> deviceName();

  /// A name to offer the person who has to choose one.
  String suggestedName();

  /// Saves the name for this device. Throws a `BatonError` of kind `badName`, whose message can
  /// be shown as it is, if there is nothing visible in it.
  Future<void> setDeviceName(String name);

  /// Starts Baton. Throws a `BatonError` that says why not.
  Future<Session> start();

  /// The last lines the library logged, oldest first, for a bug report. The library has
  /// already made them fit to paste into a public place: pairing codes, whole keys, account
  /// names in paths and public IP addresses are taken out, and so are the characters that
  /// would act on a screen. The lines can still name devices and show the addresses of the
  /// local network. It is empty if the library did not load, which is itself a reason to show
  /// the person.
  List<String> recentLog();
}

/// A Baton that is running.
abstract interface class Session {
  /// This device.
  Me get me;

  /// Events from now on. Wait for the stream before reading the lists, so that nothing that
  /// happens in between is lost: once it is here, Baton is sending to it. It ends when Baton
  /// stops.
  Future<Stream<Event>> events();

  /// The devices paired with this one.
  Future<List<Device>> devices();

  /// Where this device can be reached right now.
  Future<Network> network();

  /// Forgets a paired device. It can no longer reach this one until they are paired again.
  /// Throws a `BatonError` of kind `notPaired` if it is not paired (any more).
  Future<void> unpair(String id);

  /// Opens an invitation to pair and returns it. It can take a few seconds, because Baton
  /// gives the relay a moment to answer so that the code works from other networks. Show the
  /// ticket as a QR code and as text, and listen to [events] for the requests to pair and for
  /// the end of the offer. An offer that is already showing is withdrawn.
  Future<Offer> offerPairing();

  /// Withdraws the offer that is showing, if there is one. `Event_OfferEnded` follows.
  ///
  /// Do not call this just after saying yes to a request: the offer ends by itself then, and
  /// a withdrawal that gets there first wins.
  void cancelOffer();

  /// Answers a request to pair. Returns `false` if it is not waiting any more, because it was
  /// answered already or the offer ended; `true` does not yet mean that the devices are
  /// paired, the end of the offer says that.
  bool answerPairing(int request, {required bool accept});

  /// Pairs with the device that made [ticket]: text from the other device, or what a camera
  /// read from its QR code. The stream gives the code to compare, then the device or the
  /// reason, and ends. Failures come as items, never as errors.
  Stream<JoinProgress> joinPairing(String ticket);

  /// Gives up on a [joinPairing] that is still waiting.
  void cancelJoin();

  /// Whether this device takes files from other devices, and if not, why. It is decided when
  /// Baton starts.
  Receiving receiving();

  /// Where the files that other devices send are put, or `null` if this device takes none.
  String? get receiveFolder;

  /// Answers a request for files, which `Event_TransferRequested` brought. Returns `false` if it
  /// is not waiting any more, because it was answered already, the other device gave up, or the
  /// two minutes that the person has ran out; `true` does not yet mean that the files are here,
  /// `Event_TransferEnded` says that.
  bool answerTransfer(int request, {required bool accept});

  /// Starts sending files and folders, given by their paths, to a paired device, and returns at
  /// once with the transfer that is going. The person at the other end is asked whether to take
  /// them. Throws a `BatonError` of kind `badItem` if nothing in the paths can be sent, whose
  /// message says why, `notPaired`, or `stopped`.
  Future<FileTransfer> sendFiles(String id, List<String> paths);

  /// Sends a web link to a paired device. Throws a `BatonError` that says why not, of kind
  /// `badItem` if [url] is not a full web address, `notPaired`, `unreachable` if the device
  /// cannot be reached, or `declined` if it answers and does not take the link.
  Future<void> sendLink(String id, String url, {String? title});

  /// Sends text to a paired device. Throws a `BatonError` as [sendLink] does, with `badItem`
  /// for text that is empty or longer than the settings allow.
  Future<void> sendText(String id, String text);

  /// Checks that a paired device answers, and how its traffic gets here. Throws a
  /// `BatonError` of kind `notPaired` or `unreachable`.
  Future<Reach> ping(String id);

  /// What this device does with windows: whether it shows the windows of other devices, shares
  /// its own, and lets the person grant control of the ones it shares. Offer only what is
  /// possible. A build that has no platform code for windows says no to all three.
  WindowAbilities windowAbilities();

  /// Answers a [WindowRequest]. Returns `false` if it is not waiting any more, because it was
  /// answered already or the other device gave up. A yes starts the window if this device can
  /// show it: `Event_WindowStarted` says so, and `Event_WindowRequestEnded` says why not.
  bool answerWindow(int request, {required bool accept});

  /// Lets the person pick a window or a screen of this device, and offers it to the paired device
  /// [id]. It returns when the person at the other device has answered, which can take as long as
  /// a person takes to decide. [control] lets the other device control it, if
  /// [windowAbilities] says that this device can grant it. Throws a `BatonError` of kind
  /// `notPaired`, `unreachable`, `declined` if the device does not show windows, or `other` if
  /// this device does not share windows or a window is shared or shown already.
  Future<WindowShared> shareWindow(String id, {bool control = false});

  /// Asks the paired device [id] to start sharing its screen with this one, and shows it when it
  /// does: `Event_WindowStarted` says so, with a session that is shown here. Nobody at the other
  /// device is asked: the person there allowed this device beforehand ([setStartAccess]), or the
  /// answer is `ScreenStarted_NotStarted`, with a sentence that says why. [control] is whether the
  /// person here wishes to control the screen; the other device grants no more than the settings
  /// for this one allow, so look at `WindowSession.control` to see what was granted. It returns
  /// when the screen is shown or the answer is no. Throws a `BatonError` of kind `notPaired`,
  /// `unreachable`, `declined` if the device does not know how to be asked, or `other` if this
  /// device does not show screens or a window or screen is shared or shown already.
  Future<ScreenStarted> startScreen(String id, {bool control = false});

  /// What the paired device [id] may do at this device without a person here being asked: ask it
  /// to start sharing its screen with that device. `StartAccess.off` for a device that was just
  /// paired. Throws a `BatonError` of kind `notPaired`.
  StartAccess startAccess(String id);

  /// Lets the paired device [id] ask this device to start sharing its screen with it, to look at
  /// or to look at and control, or does not (`StartAccess.off`). Only the person at this device
  /// can say so. The device can ask only if [windowAbilities] says that this device
  /// `canGiveScreen`. A screen that is shared is not stopped by turning this off. Throws a
  /// `BatonError` of kind `notPaired`, or `storage` if the setting could not be kept.
  Future<void> setStartAccess(String id, StartAccess access);

  /// What the person at this device said of the paired device [id] hearing the sound of this
  /// device's screen: `true` that it may, `false` that it may not, and `null` when they have not
  /// said, which means that it may in a session that lets it control the screen and may not in
  /// one that lets it only look, whoever started the session. Throws a `BatonError` of kind
  /// `notPaired`.
  bool? hearSetting(String id);

  /// Says whether the paired device [id] may hear the sound of this device's screen, or takes what
  /// was said back with `null`. Only the person at this device can say so. It counts the next time
  /// the device asks to hear, also in a session that runs, but a sound that plays is not stopped
  /// until then. Throws a `BatonError` of kind `notPaired`, or `storage` if the setting could not
  /// be kept.
  Future<void> setHearSetting(String id, bool? setting);

  /// Ends the window [session], which is shown here or shared from here, and tells the other
  /// device. `Event_WindowEnded` follows. Returns whether there was such a window.
  bool endWindow(int session);

  /// Ends the window [session], which is shown here, and asks the device that shares it to leave
  /// its screen as it is when the session is over, and not lock it. It is for the person who was
  /// asked at the end of a session whose `WindowSession.leave` is set and answered that the
  /// screen stays as it is; every other end locks it, [endWindow] among them.
  /// `Event_WindowEnded` follows. Returns whether the choice was made: this call ended the
  /// session, and the other device had offered it. The other device does not answer, so a true
  /// says that it was asked and not that the screen was left: if the connection goes first, the
  /// device locks. A false is a window that is not shown here, one that has ended already, or one
  /// whose device cannot leave its screen as it is; the last of these ends as [endWindow] ends it.
  bool endWindowLeaving(int session);

  /// Gives what the person did on the picture of the window [session], which is shown here: a
  /// touch, a click, a scroll, a key. It goes to the other device soon, in a batch with others,
  /// and is done there only if the person at that device let this one control the window
  /// (`WindowSession.control`). Returns whether the event was taken. It is not for a window that
  /// is not shown here or may not be controlled, a position while no picture has been shown, a
  /// position outside the picture while no button is down, the release of something that is not
  /// down, and more than the format allows. None of that ends the window: drop the event and go
  /// on. What the other device drops comes back as `Event_WindowNotice`.
  bool windowInput(int session, WindowInput input);

  /// Lets go of every button and key that is down on the window [session], for example when the
  /// person leaves the screen with a finger on it. A window that is not shown here does
  /// nothing.
  void releaseWindowInput(int session);

  /// Tells the device that shares the window [session], which is shown here, what the people at
  /// that device may do while this one controls it: see its screen ([screen]) and use its
  /// keyboard and trackpad ([input]). It is for a window whose `WindowSession.private` is set.
  /// Both are off when the session starts, and each call says both, so the last one is how things
  /// stand: switch either on and off as often as you like, in one session. The device that
  /// shares the window locks itself and puts both back when the session ends, whatever the
  /// reason. Returns whether the message was taken, which is not so for a window that is not
  /// shown here, one that is not private, and a session whose other device has stopped reading:
  /// things are then as they were.
  bool windowLift(int session, {required bool screen, required bool input});

  /// Asks the device that shares the screen [session], which is shown here, to send its sound
  /// ([listen]), or to stop. The sound plays on this device, and the other device keeps its own
  /// speakers quiet for as long as it is sent. Sound is off when a session starts. It is for a
  /// session whose `WindowSession.sound` is set. Returns whether the request was made, which is
  /// not so for a window that is not shown here, for a window and not a whole screen, when this
  /// device plays no sound or the other sends none, and for a session that is over. How it came
  /// out comes as `Event_WindowSound`. A request to hear that is made again asks the other
  /// device to try again, which a person does after it said that it could not.
  bool hearWindow(int session, {required bool listen});

  /// What other devices have sent, newest first.
  Future<List<Received>> inbox();

  /// Removes one item from the inbox. Returns whether there was such an item.
  Future<bool> removeReceived(int number);

  /// Removes everything from the inbox. Returns how many items it held.
  Future<int> clearInbox();

  /// The address to open for a received link, or `null` if it is not a web address. A screen
  /// opens a link another device sent only if this gives an address, and then opens that
  /// address, not the text that was sent: the rule is kept in Rust, once, for every screen.
  String? webAddress(String url);

  /// The same lines as [Backend.recentLog].
  List<String> recentLog();

  /// Stops Baton and lets go of its folder.
  Future<void> stop();
}

/// A transfer of files that this device is sending.
abstract interface class FileTransfer {
  /// How far the transfer has got. This is cheap, and the latest is always there.
  FileProgress progress();

  /// Gives the transfer up, and tells the other device if it can. [outcome] then throws a
  /// `BatonError` of kind `cancelled`. It does nothing once the transfer is over.
  void cancel();

  /// Waits for the end of the transfer. Throws a `BatonError` that says why it did not come to
  /// an end that the other device answered for: `declined` if it did not take the files,
  /// `unreachable` if Baton gave up reaching it, `cancelled` if [cancel] was called, `stopped`
  /// if Baton stopped, and `notPaired` or `other` otherwise.
  Future<FilesSent> outcome();
}
