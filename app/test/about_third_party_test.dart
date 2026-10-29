// Текст для экрана «О программе» (ист. 21, R34i): компоненты и лицензии берутся
// из `THIRD_PARTY.md`. В приложение файл попадает копией в assets; копия не
// должна расходиться с оригиналом в корне репозитория.

import 'dart:io';

import 'package:braid/screens/about/third_party.dart';
import 'package:flutter_test/flutter_test.dart';

const sample = '''
# Сторонние компоненты

Что входит в сборки.
Вторая строка абзаца.

## Движки

| Компонент | Где | Лицензия | Примечание |
|---|---|---|---|
| amneziawg-go (v3) | `engines/awg-go` | MIT | ядро AmneziaWG |
| csqtt-core 2.1.9 | `engines/csqtt-core` | **PolyForm Noncommercial 1.0.0** | только для своих |

## Пакеты Dart

| Пакет | Лицензия | Зачем |
|---|---|---|
| intl | BSD-3-Clause | даты |

## Эталоны

Happ и Amnezia — только как образцы.
''';

void main() {
  group('разбор', () {
    final doc = ThirdPartyDoc.parse(sample);

    test('заголовок, вступление, разделы по порядку', () {
      expect(doc.title, 'Сторонние компоненты');
      expect(doc.intro, ['Что входит в сборки. Вторая строка абзаца.']);
      expect(doc.sections.map((s) => s.title), [
        'Движки',
        'Пакеты Dart',
        'Эталоны',
      ]);
    });

    test('таблица: шапка и строки без разметки ячеек', () {
      final table = doc.sections.first.blocks.single as ThirdPartyTable;
      expect(table.header, ['Компонент', 'Где', 'Лицензия', 'Примечание']);
      expect(table.rows, hasLength(2));
      expect(table.rows.first, [
        'amneziawg-go (v3)',
        '`engines/awg-go`',
        'MIT',
        'ядро AmneziaWG',
      ]);
    });

    test('абзац в разделе остаётся текстом', () {
      final text = doc.sections.last.blocks.single as ThirdPartyText;
      expect(text.text, 'Happ и Amnezia — только как образцы.');
    });

    test('разметка снимается для показа', () {
      expect(
        plainText('**PolyForm Noncommercial 1.0.0**'),
        'PolyForm Noncommercial 1.0.0',
      );
      expect(
        plainText('`engines/awg-go` → `libawg`'),
        'engines/awg-go → libawg',
      );
    });

    test('лицензия PolyForm Noncommercial отмечается особо', () {
      expect(isNoncommercial('**PolyForm Noncommercial 1.0.0**'), isTrue);
      expect(isNoncommercial('MIT'), isFalse);
      expect(isNoncommercial('Apache-2.0'), isFalse);
    });
  });

  group('настоящий файл', () {
    // Тесты запускаются из корня `app/`.
    final original = File('../THIRD_PARTY.md');
    final copy = File('assets/third_party.md');

    test('копия в assets совпадает с THIRD_PARTY.md в корне', () {
      expect(
        copy.readAsStringSync(),
        original.readAsStringSync(),
        reason: 'обнови копию: cp ../THIRD_PARTY.md assets/third_party.md',
      );
    });

    test('csqtt-core в списке и с лицензией PolyForm Noncommercial', () {
      final doc = ThirdPartyDoc.parse(copy.readAsStringSync());
      final rows = [
        for (final section in doc.sections)
          for (final block in section.blocks)
            if (block is ThirdPartyTable) ...block.rows,
      ];
      final csqtt = rows.where((row) => row.first.contains('csqtt-core'));
      expect(csqtt, isNotEmpty);
      expect(
        csqtt.any((row) => row.any(isNoncommercial)),
        isTrue,
        reason: 'ограничение лицензии должно быть видно на экране',
      );
    });
  });
}
