// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'content_box.dart';

/// The first screen: the person names this device. Other devices show the name when they pair.
class NameScreen extends StatefulWidget {
  const NameScreen({
    required this.suggestion,
    required this.onChosen,
    super.key,
  });

  /// What the field starts with.
  final String suggestion;

  /// Saves the name. Returns what to tell the person if it cannot be used, and `null` once it
  /// is saved.
  final Future<String?> Function(String name) onChosen;

  @override
  State<NameScreen> createState() => _NameScreenState();
}

class _NameScreenState extends State<NameScreen> {
  late final TextEditingController _name = TextEditingController(
    text: widget.suggestion,
  );
  bool _saving = false;
  String? _refused;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (_saving) return;
    setState(() {
      _saving = true;
      _refused = null;
    });
    final refused = await widget.onChosen(_name.text);
    if (!mounted) return;
    setState(() {
      _saving = false;
      _refused = refused;
    });
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Scaffold(
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
            children: [
              Text('Name this device', style: text.headlineMedium),
              const SizedBox(height: 12),
              Text(
                'Your other devices show this name when you pair with them. '
                'Use something you will recognise, like “Kitchen iPad”.',
                style: text.bodyLarge,
              ),
              const SizedBox(height: 24),
              TextField(
                controller: _name,
                autofocus: true,
                enabled: !_saving,
                autocorrect: false,
                textCapitalization: TextCapitalization.words,
                textInputAction: TextInputAction.done,
                // The longest name that other devices will show whole.
                inputFormatters: [LengthLimitingTextInputFormatter(64)],
                onSubmitted: (_) => unawaited(_submit()),
                decoration: InputDecoration(
                  labelText: 'Device name',
                  border: const OutlineInputBorder(),
                  errorText: _refused,
                  errorMaxLines: 3,
                ),
              ),
              const SizedBox(height: 24),
              FilledButton(
                onPressed: _saving ? null : () => unawaited(_submit()),
                child: _saving
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Text('Continue'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
