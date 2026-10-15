// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

/// Keeps the content of a screen to a comfortable width, on an iPad or in a Mac window.
class ContentBox extends StatelessWidget {
  const ContentBox({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.topCenter,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: child,
      ),
    );
  }
}
