// Ответ скрипта из встроенного браузера Android приходит JSON-строкой, на
// macOS — значением как есть; сценарии входа и капчи ждут обычную строку.

import 'package:braid/services/captcha/in_app_web_launcher.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('строка в кавычках (Android) раскрывается, экранирование снимается', () {
    expect(plainJavaScriptResult('"hooked"'), 'hooked');
    expect(
      plainJavaScriptResult(r'"{\"success_token\":\"abc\"}"'),
      '{"success_token":"abc"}',
    );
    expect(
      plainJavaScriptResult(r'"строка\nс переводом"'),
      'строка\nс переводом',
    );
    expect(plainJavaScriptResult('""'), '');
  });

  test('обычная строка (iOS, macOS) остаётся как есть', () {
    expect(
      plainJavaScriptResult('https://oauth.vk.ru/blank.html#a=1'),
      'https://oauth.vk.ru/blank.html#a=1',
    );
  });

  test('null скрипта — пустая строка, а не слово «null»', () {
    expect(plainJavaScriptResult('null'), '');
  });

  test('не строка приводится к тексту', () {
    expect(plainJavaScriptResult(42), '42');
    expect(plainJavaScriptResult(true), 'true');
  });
}
