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
        Reach,
        Received,
        ReceivedItem,
        ReceivedItem_Link,
        ReceivedItem_Text,
        Route;

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
