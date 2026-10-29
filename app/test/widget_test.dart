// Экран-заглушка: Material 3, русская локаль, кнопка «Подключить» недоступна
// без подписки, версия ядра видна. Мост Rust не трогаем — версия передаётся снаружи.

import 'package:braid/app.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('заглушка показывает название, кнопку и версию ядра', (tester) async {
    await tester.pumpWidget(const BraidApp(coreVersion: '0.1.0-test'));
    await tester.pumpAndSettle();

    expect(find.text('Braid'), findsOneWidget);
    expect(find.text('Подключить'), findsOneWidget);
    final button = tester.widget<FilledButton>(find.byType(FilledButton));
    expect(button.onPressed, isNull, reason: 'без подписки подключаться нечем');
    expect(find.text('Вставь ссылку или отсканируй QR'), findsOneWidget);
    expect(find.textContaining('0.1.0-test'), findsOneWidget);

    final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(app.theme?.useMaterial3, isTrue);
    expect(Localizations.localeOf(tester.element(find.text('Braid'))).languageCode, 'ru');
  });
}
