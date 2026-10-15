// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:url_launcher/url_launcher.dart';

/// Opens a web address in the person's web browser. Returns whether that worked. Tests give the
/// screens one of their own, so that nothing opens a browser.
typedef OpenLink = Future<bool> Function(String address);

Future<bool> _launch(Uri uri) =>
    launchUrl(uri, mode: LaunchMode.externalApplication);

/// Opens [address] in the web browser of this device, outside the app.
///
/// [address] is what `Session.webAddress` gave back for a link that another device sent, never
/// the link as it arrived. This checks the scheme once more, on the way out, because the
/// parser here is not the one that checked it there. [launch] is what hands the address to
/// the system; a test gives one of its own.
Future<bool> openInBrowser(
  String address, {
  @visibleForTesting Future<bool> Function(Uri uri) launch = _launch,
}) async {
  try {
    final uri = Uri.parse(address);
    final web = uri.isScheme('http') || uri.isScheme('https');
    if (!web || uri.host.isEmpty) return false;
    return await launch(uri);
  } on Object catch (e) {
    // A link can hold a sign-in token, so only what went wrong is written down, not where.
    debugPrint('Baton: a link did not open: ${e.runtimeType}');
    return false;
  }
}
