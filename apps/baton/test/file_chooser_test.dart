// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

import 'package:baton/src/file_chooser.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

/// A folder that stands for the temporary folder of the app, and a folder beside it that is not,
/// both made for one test and deleted after it.
class _Folders {
  _Folders._(this.root, this.temporary, this.elsewhere);

  static Future<_Folders> make() async {
    final root = await Directory.systemTemp.createTemp('baton-chooser-test-');
    addTearDown(() => root.delete(recursive: true));
    final temporary = await Directory('${root.path}/tmp').create();
    final elsewhere = await Directory('${root.path}/Documents').create();
    return _Folders._(root, temporary, elsewhere);
  }

  final Directory root;
  final Directory temporary;
  final Directory elsewhere;

  /// A file with something in it, in [folder] and not further down.
  Future<File> file(Directory folder, String name) async {
    final file = File('${folder.path}/$name');
    await file.writeAsString('x');
    return file;
  }

  SystemFileChooser chooser([TargetPlatform platform = TargetPlatform.iOS]) =>
      SystemFileChooser(platform: platform, temporaryFolder: temporary.path);
}

void main() {
  group('lets go of what was chosen', () {
    test(
      'by deleting the copies that the picker made in the temporary folder',
      () async {
        final folders = await _Folders.make();
        final first = await folders.file(folders.temporary, 'holiday.jpg');
        final second = await folders.file(folders.temporary, 'notes.txt');

        await folders.chooser().release([first.path, second.path]);

        expect(first.existsSync(), isFalse);
        expect(second.existsSync(), isFalse);
        expect(folders.temporary.existsSync(), isTrue);
      },
    );

    test('also when a copy is in a folder of its own', () async {
      final folders = await _Folders.make();
      final inside = await Directory('${folders.temporary.path}/picked')
          .create();
      final copy = await folders.file(inside, 'holiday.jpg');

      await folders.chooser().release([copy.path]);

      expect(copy.existsSync(), isFalse);
    });

    test('and goes on when one of them is gone already', () async {
      final folders = await _Folders.make();
      final gone = File('${folders.temporary.path}/gone.jpg');
      final there = await folders.file(folders.temporary, 'there.jpg');

      await folders.chooser().release([gone.path, there.path]);

      expect(there.existsSync(), isFalse);
    });

    test('and leaves what is not in the temporary folder', () async {
      final folders = await _Folders.make();
      final original = await folders.file(folders.elsewhere, 'holiday.jpg');

      await folders.chooser().release([original.path]);

      expect(original.existsSync(), isTrue);
    });

    test('and leaves a folder whose name only begins like its name', () async {
      final folders = await _Folders.make();
      final beside = await Directory('${folders.temporary.path}-elsewhere')
          .create();
      final original = await folders.file(beside, 'holiday.jpg');

      await folders.chooser().release([original.path]);

      expect(original.existsSync(), isTrue);
    });

    test('and leaves a path that climbs out of the temporary folder', () async {
      final folders = await _Folders.make();
      final original = await folders.file(folders.elsewhere, 'holiday.jpg');

      await folders.chooser().release([
        '${folders.temporary.path}/../Documents/holiday.jpg',
      ]);

      expect(original.existsSync(), isTrue);
    });

    test('and leaves a folder, even one in the temporary folder', () async {
      final folders = await _Folders.make();
      final folder = await Directory('${folders.temporary.path}/Photos')
          .create();
      final inside = await folders.file(folder, 'a.jpg');

      await folders.chooser().release([folder.path, folders.temporary.path]);

      expect(inside.existsSync(), isTrue);
      expect(folders.temporary.existsSync(), isTrue);
    });

    test('and does not follow a link out of the temporary folder', () async {
      final folders = await _Folders.make();
      final original = await folders.file(folders.elsewhere, 'holiday.jpg');
      final link = Link('${folders.temporary.path}/link.jpg');
      await link.create(original.path);
      final folderLink = Link('${folders.temporary.path}/documents');
      await folderLink.create(folders.elsewhere.path);

      await folders.chooser().release([
        link.path,
        '${folderLink.path}/holiday.jpg',
      ]);

      expect(original.existsSync(), isTrue);
      expect(
        link.existsSync(),
        isTrue,
        reason: 'a link is not a copy that the picker made',
      );
    }, skip: Platform.isWindows);

    test(
      'and a temporary folder that is itself reached through a link',
      () async {
        final folders = await _Folders.make();
        final copy = await folders.file(folders.temporary, 'holiday.jpg');
        // The system names the same folder in more than one way, such as /var and /private/var.
        final alias = Link('${folders.root.path}/alias');
        await alias.create(folders.temporary.path);

        await SystemFileChooser(
          platform: TargetPlatform.iOS,
          temporaryFolder: alias.path,
        ).release([copy.path]);

        expect(copy.existsSync(), isFalse);
      },
      skip: Platform.isWindows,
    );

    test(
      'and does nothing on a Mac, where the person chooses their own files',
      () async {
        final folders = await _Folders.make();
        final own = await folders.file(folders.temporary, 'holiday.jpg');

        await folders.chooser(TargetPlatform.macOS).release([own.path]);

        expect(own.existsSync(), isTrue);
      },
    );

    test('and has nothing to do for nothing', () async {
      final folders = await _Folders.make();
      final copy = await folders.file(folders.temporary, 'holiday.jpg');

      await folders.chooser().release(const []);

      expect(copy.existsSync(), isTrue);
    });

    test('and does not throw when the temporary folder is not there', () async {
      final folders = await _Folders.make();
      final copy = await folders.file(folders.temporary, 'holiday.jpg');

      await SystemFileChooser(
        platform: TargetPlatform.iOS,
        temporaryFolder: '${folders.root.path}/not-there',
      ).release([copy.path]);

      expect(copy.existsSync(), isTrue);
    });
  });

  group('chooses', () {
    test('folders only on a Mac', () {
      for (final platform in TargetPlatform.values) {
        expect(
          SystemFileChooser(platform: platform).canChooseFolders,
          platform == TargetPlatform.macOS,
          reason: '$platform',
        );
      }
    });
  });
}
