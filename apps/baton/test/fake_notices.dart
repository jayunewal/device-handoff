// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/notice_access.dart';

/// Says what a test wants the system to say about showing notifications, and notes what it was
/// asked to show.
class FakeNotices extends NoticeKeeper {
  FakeNotices([this.access = NoticeAccess.allowed]);

  /// What `state` gives.
  NoticeAccess access;

  /// What the person says when the system asks them: `request` gives it, and `state` gives it from
  /// then on. It is asked only of a system that has not been asked yet.
  NoticeAccess answer = NoticeAccess.allowed;

  /// How many times `state` was asked.
  int asked = 0;

  /// How many times `request` was called.
  int requests = 0;

  /// What `post` was given, in order.
  final List<({String title, String body})> posted = [];

  /// Whether `post` says that the system took the notification.
  bool taken = true;

  /// What `post` throws, if it should fail.
  Object? postFailure;

  @override
  Future<NoticeAccess> state() async {
    asked++;
    return access;
  }

  @override
  Future<NoticeAccess> request() async {
    requests++;
    if (access == NoticeAccess.needed) access = answer;
    return access;
  }

  @override
  Future<bool> post({required String title, required String body}) async {
    final failure = postFailure;
    if (failure != null) throw failure;
    posted.add((title: title, body: body));
    return taken;
  }
}
