// SPDX-License-Identifier: Apache-2.0

// Two Batons on this computer pair through the controllers that the pairing screens use, with
// the ticket handed over as a camera would hand it. Nothing is faked but the camera. It needs the
// library built for this computer; see bridge_test.dart.

import 'package:baton/src/backend.dart';
import 'package:baton/src/pairing_controller.dart';
import 'package:baton/src/qr.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

Future<Session> _start(String name) async {
  final backend = backendIn(await deviceFolder());
  await backend.setDeviceName(name);
  final session = await backend.start();
  addTearDown(session.stop);
  return session;
}

/// An offer on [session] that is showing, and the text a camera reads from its QR code, which is
/// the ticket. That the code reads back as the ticket is tried in qr_test.dart, on tickets that
/// stay the same: the decoder in these tests fails on one valid code in a hundred, and the ticket
/// of a live Baton is different every time.
Future<(OfferController, String)> _showCode(Session session) async {
  final offer = OfferController(session);
  addTearDown(offer.dispose);
  await offer.start();
  expect(offer.phase, OfferPhase.showing);
  final ticket = offer.offer!.ticket;
  expect(
    () => ticketCode(ticket),
    returnsNormally,
    reason: 'it fits a QR code',
  );
  return (offer, ticket);
}

void main() {
  test('a device shows a code, another reads it, and they pair', () async {
    final mac = await _start('Desk Mac');
    final phone = await _start('Anna’s iPhone');
    final (offer, seen) = await _showCode(mac);

    final join = JoinController(phone);
    addTearDown(join.dispose);
    join.scanned(seen);
    expect(join.phase, JoinPhase.connecting);
    await until(
      join,
      () => join.phase == JoinPhase.comparing,
      describe: () => '${join.phase} ${join.problem}',
    );
    await until(offer, () => offer.phase == OfferPhase.asking);

    // The two screens show the same six digits, and the Mac says who is asking.
    expect(join.code, matches(RegExp(r'^\d{3} \d{3}$')));
    expect(offer.request!.code, join.code);
    expect(offer.request!.name, 'Anna’s iPhone');
    expect(offer.request!.deviceId, phone.me.id);
    expect(offer.request!.fingerprint, phone.me.fingerprint);

    offer.answer(accept: true);
    expect(offer.phase, OfferPhase.answering);
    await until(offer, () => offer.phase == OfferPhase.paired);
    await until(join, () => join.phase == JoinPhase.paired);
    expect(offer.device!.id, phone.me.id);
    expect(offer.device!.name, 'Anna’s iPhone');
    expect(join.device!.id, mac.me.id);
    expect(join.device!.name, 'Desk Mac');
    expect((await mac.devices()).map((d) => d.id), [phone.me.id]);
    expect((await phone.devices()).map((d) => d.id), [mac.me.id]);

    // Closing the screens now leaves the pairing as it is.
    offer.dispose();
    join.dispose();
    expect((await mac.devices()).map((d) => d.id), [phone.me.id]);
  });

  test('a device that is turned down is told, and nothing is kept', () async {
    final mac = await _start('Desk Mac');
    final phone = await _start('Anna’s iPhone');
    final (offer, seen) = await _showCode(mac);
    final join = JoinController(phone);
    addTearDown(join.dispose);
    join.join(seen);
    await until(offer, () => offer.phase == OfferPhase.asking);

    offer.answer(accept: false);

    await until(offer, () => offer.phase == OfferPhase.over);
    expect(offer.outcome, isA<OfferOutcome_Declined>());
    await until(join, () => join.phase == JoinPhase.failed);
    expect(join.problem, isNotEmpty);
    expect(join.problem, isNot(contains('\n')));
    expect(await mac.devices(), isEmpty);
    expect(await phone.devices(), isEmpty);

    // The person asks for a new code, and that one works.
    await offer.again();
    expect(offer.phase, OfferPhase.showing);
    join.tryAgain();
    expect(join.phase, JoinPhase.entering);
    join.join(offer.offer!.ticket);
    await until(offer, () => offer.phase == OfferPhase.asking);
    offer.answer(accept: true);
    await until(offer, () => offer.phase == OfferPhase.paired);
    await until(join, () => join.phase == JoinPhase.paired);
  });

  test('a code that is not Baton\'s is turned down in words', () async {
    final phone = await _start('Anna’s iPhone');
    final join = JoinController(phone);
    addTearDown(join.dispose);

    join.scanned('https://example.com/menu');
    await until(join, () => join.problem.isNotEmpty);

    expect(join.phase, JoinPhase.entering, reason: 'the camera stays on');
    expect(join.problem, contains('Baton'));
    expect(join.problem, isNot(contains('\n')));
  });

  test('a code that the screen has left no longer works', () async {
    final mac = await _start('Desk Mac');
    final phone = await _start('Anna’s iPhone');
    final (offer, seen) = await _showCode(mac);
    offer.dispose();

    final join = JoinController(phone);
    addTearDown(join.dispose);
    join.join(seen);

    await until(
      join,
      () => join.phase == JoinPhase.failed,
      describe: () => '${join.phase} ${join.problem}',
    );
    expect(await mac.devices(), isEmpty);
    expect(await phone.devices(), isEmpty);
  });

  test('a device that gives up spends the code, and the Mac is told', () async {
    final mac = await _start('Desk Mac');
    final phone = await _start('Anna’s iPhone');
    final (offer, seen) = await _showCode(mac);
    final join = JoinController(phone);
    join.join(seen);
    await until(offer, () => offer.phase == OfferPhase.asking);

    // The person on the phone closes the screen before the Mac has answered.
    join.dispose();

    await until(
      offer,
      () => offer.phase == OfferPhase.over,
      describe: () => offer.phase.name,
    );
    expect(offer.outcome, isA<OfferOutcome_Abandoned>());
    expect(offer.request, isNull, reason: 'the question is off the screen');
    expect(await mac.devices(), isEmpty);
  });
}
