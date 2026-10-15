// SPDX-License-Identifier: Apache-2.0

// Two Batons on this computer send to each other through the controllers that the send and inbox
// screens use. Nothing is faked. It needs the library built for this computer; see
// bridge_test.dart.

import 'package:baton/src/backend.dart';
import 'package:baton/src/inbox_controller.dart';
import 'package:baton/src/pairing_controller.dart';
import 'package:baton/src/send_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

Future<Session> _start(String name) async {
  final backend = backendIn(await deviceFolder());
  await backend.setDeviceName(name);
  final session = await backend.start();
  addTearDown(session.stop);
  return session;
}

/// Two new Batons that are paired with each other: the first shows a code, the second reads it.
Future<(Session, Session)> _paired() async {
  final mac = await _start('Desk Mac');
  final phone = await _start('Anna’s iPhone');
  final offer = OfferController(mac);
  addTearDown(offer.dispose);
  await offer.start();
  final join = JoinController(phone);
  addTearDown(join.dispose);
  join.join(offer.offer!.ticket);
  await until(offer, () => offer.phase == OfferPhase.asking);
  offer.answer(accept: true);
  await until(offer, () => offer.phase == OfferPhase.paired);
  await until(join, () => join.phase == JoinPhase.paired);
  return (mac, phone);
}

/// The controllers of the Mac's inbox, and of sending from the phone to the Mac.
Future<(InboxController, SendController)> _screens(
  Session mac,
  Session phone,
) async {
  final inbox = InboxController(mac);
  addTearDown(inbox.dispose);
  await inbox.start();
  final toMac = SendController(phone, (await phone.devices()).single);
  addTearDown(toMac.dispose);
  return (inbox, toMac);
}

String _text(Received received) => (received.item as ReceivedItem_Text).text;

void main() {
  test(
    'a link and some text arrive in the other inbox, newest first',
    () async {
      final (mac, phone) = await _paired();
      final (inbox, toMac) = await _screens(mac, phone);
      final arrived = <Received>[];
      inbox.arrivals.listen(arrived.add);
      expect(inbox.items, isEmpty);

      expect(await toMac.send('  HTTPS://Example.com/plan \n'), isTrue);
      expect(toMac.outcome!.message, 'Sent the link to Desk Mac.');
      // Text arrives exactly as it was typed: line ends, spaces and all.
      const typed = ' Zoë’s door code:\r\n4711 🎉 ';
      expect(await toMac.send(typed), isTrue);
      expect(toMac.outcome!.message, 'Sent the text to Desk Mac.');

      await until(
        inbox,
        () => inbox.items!.length == 2,
        describe: () => 'items: ${inbox.items?.length}, ${inbox.problem}',
      );
      final [newest, oldest] = inbox.items!;
      expect(_text(newest), typed);
      expect(oldest.item, isA<ReceivedItem_Link>());
      expect(
        (oldest.item as ReceivedItem_Link).url,
        'https://example.com/plan',
      );
      expect(newest.fromName, 'Anna’s iPhone');
      expect(newest.fromId, phone.me.id);
      expect(newest.number, greaterThan(oldest.number));

      expect(inbox.unseen, 2, reason: 'both came while the app was open');
      expect(arrived.map((received) => received.number), [
        oldest.number,
        newest.number,
      ]);
      // A link is opened by the address the library wrote out, and text is never opened.
      expect(inbox.addressToOpen(oldest), 'https://example.com/plan');
      expect(inbox.addressToOpen(newest), isNull);
    },
  );

  test('what was in the inbox before the app opened is not new', () async {
    final (mac, phone) = await _paired();
    final (first, toMac) = await _screens(mac, phone);
    expect(await toMac.send('left for later'), isTrue);
    await until(first, () => first.items!.length == 1);

    final reopened = InboxController(mac);
    addTearDown(reopened.dispose);
    await reopened.start();

    expect(reopened.items!.map(_text), ['left for later']);
    expect(reopened.unseen, 0);
  });

  test('takes items out, one or all, and never numbers two the same', () async {
    final (mac, phone) = await _paired();
    final (inbox, toMac) = await _screens(mac, phone);
    for (final text in ['one', 'two', 'three']) {
      expect(await toMac.send(text), isTrue);
    }
    await until(inbox, () => inbox.items!.length == 3);
    expect(inbox.items!.map(_text), ['three', 'two', 'one']);
    final highest = inbox.items!.first.number;

    expect(await inbox.remove(inbox.items![1]), isNull);
    await until(inbox, () => inbox.items!.length == 2);
    expect(inbox.items!.map(_text), ['three', 'one']);
    expect((await mac.inbox()).map(_text), ['three', 'one']);

    expect(await inbox.clear(), isNull);
    await until(inbox, () => inbox.items!.isEmpty);
    expect(await mac.inbox(), isEmpty);
    expect(inbox.unseen, 0);

    expect(await toMac.send('after'), isTrue);
    await until(inbox, () => inbox.items!.length == 1);
    expect(
      inbox.items!.single.number,
      greaterThan(highest),
      reason: 'a number that was used is not used again',
    );
  });

  test('says whether the other Baton answers, and how fast', () async {
    final (mac, phone) = await _paired();
    final (_, toMac) = await _screens(mac, phone);

    await toMac.check();

    expect(toMac.outcome!.ok, isTrue, reason: toMac.outcome!.message);
    expect(
      toMac.outcome!.message,
      matches(RegExp(r'^Desk Mac answered in \d+ ms')),
    );
  });

  test('the library and the app agree on what is a web address', () async {
    final (_, phone) = await _paired();

    expect(
      phone.webAddress(' HTTPS://Example.com/a?b=c#d '),
      'https://example.com/a?b=c#d',
    );
    expect(phone.webAddress('http://example.com'), 'http://example.com/');
    for (final other in [
      'tel:+15551234',
      'mailto:anna@example.com',
      'file:///etc/hosts',
      'javascript:alert(1)',
      'example.com/a',
      'not a link',
      'https://',
      '',
    ]) {
      expect(phone.webAddress(other), isNull, reason: other);
    }
  });

  test(
    'a send to a device that is not paired fails at once, in words',
    () async {
      final (_, phone) = await _paired();
      final stranger = Device(
        id: 'ab' * 32,
        name: 'Stranger',
        platform: Platform.other,
        fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
        pairedAt: 0,
      );
      final toStranger = SendController(phone, stranger);
      addTearDown(toStranger.dispose);

      await expectLater(
        phone.sendText(stranger.id, 'hello'),
        throwsA(
          isA<BatonError>().having((e) => e.kind, 'kind', ErrorKind.notPaired),
        ),
      );
      expect(await toStranger.send('hello'), isFalse);

      expect(toStranger.outcome!.ok, isFalse);
      expect(toStranger.outcome!.message, endsWith('.'));
      expect(toStranger.outcome!.message, isNot(contains('\n')));
      expect(toStranger.sending, isFalse);
    },
  );
}
