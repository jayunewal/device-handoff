// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/send_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

final _kitchenIpad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: [_kitchenIpad],
);

SendController _controller(FakeSession session) {
  final controller = SendController(session, _kitchenIpad);
  addTearDown(controller.dispose);
  return controller;
}

void main() {
  test(
    'a web address on its own goes as a link, written out in full',
    () async {
      final session = _session();
      final controller = _controller(session);

      expect(
        controller.linkIn('  HTTPS://Example.com/a  '),
        'https://example.com/a',
      );
      expect(await controller.send('  HTTPS://Example.com/a \n'), isTrue);

      expect(session.sent, [
        (
          id: _kitchenIpad.id,
          url: 'https://example.com/a',
          title: null,
          text: null,
        ),
      ]);
      expect(controller.outcome!.message, 'Sent the link to Kitchen iPad.');
      expect(controller.outcome!.ok, isTrue);
    },
  );

  test('anything else goes as text, as it was typed', () async {
    final session = _session();
    final controller = _controller(session);
    const notes =
        '  Look at https://example.com/a for the plan\n\nand the rest  ';

    expect(
      controller.linkIn(notes),
      isNull,
      reason: 'there are words around it',
    );
    expect(controller.linkIn('example.com/a'), isNull, reason: 'no https://');
    expect(controller.linkIn('mailto:anna@example.com'), isNull);
    expect(controller.linkIn('tel:+15551234'), isNull);
    expect(controller.linkIn('file:///etc/hosts'), isNull);
    expect(controller.linkIn('https://example.com/a b'), isNull);
    expect(controller.linkIn(''), isNull);

    expect(await controller.send(notes), isTrue);
    expect(await controller.send('tel:+15551234'), isTrue);

    expect(session.sent.map((sent) => (sent.url, sent.text)), [
      (null, notes),
      (null, 'tel:+15551234'),
    ]);
    expect(controller.outcome!.message, 'Sent the text to Kitchen iPad.');
  });

  test('nothing is sent for a box with nothing in it', () async {
    final session = _session();
    final controller = _controller(session);

    expect(await controller.send(''), isFalse);
    expect(await controller.send('  \n\t '), isFalse);

    expect(session.sent, isEmpty);
    expect(controller.outcome, isNull);
  });

  test('says that it is sending, and sends only once at a time', () async {
    final session = _session()..holdSend();
    final controller = _controller(session);
    final states = <bool>[];
    controller.addListener(() => states.add(controller.sending));

    final first = controller.send('hello');
    final second = await controller.send('hello again');
    expect(controller.sending, isTrue);
    expect(second, isFalse);

    session.releaseSend();
    expect(await first, isTrue);

    expect(controller.sending, isFalse);
    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(states, [true, false]);
  });

  test(
    'says why it could not send, in a sentence, and what was typed stays',
    () async {
      final session = _session()
        ..sendFailure = const BatonError(
          kind: ErrorKind.unreachable,
          message:
              'could not reach Kitchen iPad (connecting to the other device: timed out). '
              'Baton has to be open on it',
        );
      final controller = _controller(session);

      expect(await controller.send('hello'), isFalse);

      expect(
        controller.outcome!.message,
        'Could not reach Kitchen iPad (connecting to the other device: timed out). '
        'Baton has to be open on it.',
      );
      expect(controller.outcome!.ok, isFalse);
      expect(controller.sending, isFalse);
    },
  );

  test('forgets what happened once the text is changed', () async {
    final session = _session();
    final controller = _controller(session);
    var heard = 0;
    controller.addListener(() => heard++);

    controller.edited();
    expect(heard, 0, reason: 'nothing to forget');

    await controller.send('hello');
    final afterSend = heard;
    expect(controller.outcome, isNotNull);

    controller.edited();
    expect(controller.outcome, isNull);
    expect(heard, afterSend + 1);
  });

  test('a new send replaces what was said about the last one', () async {
    final session = _session();
    final controller = _controller(session);

    session.sendFailure = const BatonError(
      kind: ErrorKind.declined,
      message: 'Kitchen iPad did not take it',
    );
    expect(await controller.send('hello'), isFalse);
    expect(controller.outcome!.ok, isFalse);

    session.sendFailure = null;
    expect(await controller.send('hello'), isTrue);
    expect(controller.outcome!.ok, isTrue);
  });

  test('checks that the device answers, and how', () async {
    final session = _session();
    final controller = _controller(session);

    await controller.check();

    expect(session.pinged, [_kitchenIpad.id]);
    expect(
      controller.outcome!.message,
      'Kitchen iPad answered in 24 ms, over your local network.',
    );
    expect(controller.outcome!.ok, isTrue);
    expect(controller.checking, isFalse);
  });

  test('says when the device does not answer', () async {
    final session = _session()
      ..pingFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'could not reach Kitchen iPad. Baton has to be open on it',
      );
    final controller = _controller(session);

    await controller.check();

    expect(
      controller.outcome!.message,
      'Could not reach Kitchen iPad. Baton has to be open on it.',
    );
    expect(controller.outcome!.ok, isFalse);
  });

  test('a send that ends after the screen is gone does no harm', () async {
    final session = _session()..holdSend();
    final controller = SendController(session, _kitchenIpad);

    final sending = controller.send('hello');
    controller.dispose();
    session.releaseSend();

    expect(await sending, isTrue);
  });
}
