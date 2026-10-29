// Файл со своими правилами (ист. 52): что пишется и как читается обратно.

import 'dart:convert';

import 'package:braid/services/rules_transfer.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('записанное читается обратно без потерь', () {
    final text = RulesTransfer.encode(
      domains: ['example.org', '*.cdn.example.net'],
      subnets: ['10.20.0.0/16'],
    );
    final plan = RulesTransfer.decode(text);
    expect(plan.domains, ['example.org', '*.cdn.example.net']);
    expect(plan.subnets, ['10.20.0.0/16']);
    expect(plan.rejected, isEmpty);
  });

  test('в файле — отмеченный формат и версия, чтобы чужой JSON не принять за правила', () {
    final json = jsonDecode(
      RulesTransfer.encode(domains: [], subnets: []),
    ) as Map<String, dynamic>;
    expect(json['format'], 'braid-rules');
    expect(json['version'], 1);
  });

  test(
    'записи приводятся к канону, повторы схлопываются, плохие считаются',
    () {
      final plan = RulesTransfer.decode(
        '{"format":"braid-rules","version":1,'
        '"domains":["https://x.y/путь","X.Y","localhost"],'
        '"subnets":["10.1.2.3/8","10.0.0.0/8","нет"]}',
      );
      expect(plan.domains, ['x.y']);
      expect(plan.subnets, ['10.0.0.0/8']);
      expect(plan.rejected, ['localhost', 'нет']);
    },
  );

  test('пустые разделы допустимы', () {
    final plan = RulesTransfer.decode('{"format":"braid-rules","version":1}');
    expect(plan.domains, isEmpty);
    expect(plan.subnets, isEmpty);
  });

  test('не тот файл — понятная ошибка', () {
    for (final text in [
      '',
      '{не json',
      '[]',
      '{"что-то":1}',
      '{"format":"другое","version":1}',
    ]) {
      expect(
        () => RulesTransfer.decode(text),
        throwsA(
          isA<FormatException>().having(
            (e) => e.message,
            'message',
            'Это не файл правил Braid',
          ),
        ),
        reason: text,
      );
    }
    expect(
      () => RulesTransfer.decode(
        '{"format":"braid-rules","version":1,"domains":"x.y"}',
      ),
      throwsFormatException,
    );
  });

  test('другая версия формата не читается вслепую', () {
    expect(
      () => RulesTransfer.decode('{"format":"braid-rules","version":2}'),
      throwsA(
        isA<FormatException>().having(
          (e) => e.message,
          'message',
          'Файл правил другой версии — обнови Braid',
        ),
      ),
    );
  });
}
