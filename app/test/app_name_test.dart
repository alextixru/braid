// Имя приложения — одна константа `appName` (`lib/app.dart`): тексты
// интерфейса берут его оттуда, и смена одной строки переименовывает
// приложение целиком. Тест читает исходники `lib/` (кроме сгенерированного
// `lib/src/rust/`) и падает на литерале «Braid» в коде вне этого определения.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Слово «Braid» целиком: не часть идентификатора вроде `BraidController`
/// и не `package:braid`.
final _word = RegExp(r'(?<![A-Za-z0-9_])Braid(?![A-Za-z0-9_])');

final _definition = RegExp(r"^const String appName = 'Braid';$");

void main() {
  test('литерал «Braid» в lib/ — только в определении appName', () {
    final files =
        Directory('lib')
            .listSync(recursive: true)
            .whereType<File>()
            .where((f) => f.path.endsWith('.dart'))
            .where(
              (f) => !f.path.contains(
                '${Platform.pathSeparator}src'
                '${Platform.pathSeparator}rust${Platform.pathSeparator}',
              ),
            )
            .toList()
          ..sort((a, b) => a.path.compareTo(b.path));
    expect(files, isNotEmpty, reason: 'тест запускается из корня app/');

    final offenders = <String>[];
    var definitions = 0;
    for (final file in files) {
      final lines = file.readAsLinesSync();
      for (var i = 0; i < lines.length; i++) {
        final line = lines[i].trim();
        // Комментарии — не тексты интерфейса.
        if (line.startsWith('//')) continue;
        if (_definition.hasMatch(line)) {
          definitions++;
          continue;
        }
        if (_word.hasMatch(line)) {
          offenders.add('${file.path}:${i + 1}: $line');
        }
      }
    }
    expect(definitions, 1, reason: 'ровно одно определение appName');
    expect(
      offenders,
      isEmpty,
      reason:
          'имя приложения зашито литералом — возьми appName:\n'
          '${offenders.join('\n')}',
    );
  });
}
