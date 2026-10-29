// Проверка ввода на экране «Движки → CSQTT» (ист. 14, 15): ссылка на звонок
// VK или голый хеш. Правило то же, что у ядра (`parse_call_hash` в
// `braid-engine-csqtt`), поэтому ожидаемые значения — из его тестов: экран
// не должен принять то, что ядро потом отвергнет, и наоборот.

import 'package:braid/services/vk/call_hash.dart';
import 'package:flutter_test/flutter_test.dart';

const hash = 'kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA';

void main() {
  test('ссылки на звонок приводятся к хешу', () {
    for (final link in [
      'https://vk.com/call/join/$hash',
      'https://vk.ru/call/join/$hash/',
      'http://m.vk.com/call/join/$hash?utm=1#x',
      'vk.com/call/join/$hash',
      '  <https://vk.com/call/join/$hash>  ',
    ]) {
      expect(parseCallHash(link), hash, reason: link);
    }
  });

  test('голый хеш принимается как есть', () {
    expect(parseCallHash(hash), hash);
    expect(parseCallHash('abcdefghijklmnop'), 'abcdefghijklmnop');
  });

  test('всё остальное — не ссылка на звонок', () {
    for (final input in [
      '',
      '   ',
      'https://example.com/call/join/abc',
      'https://vk.com/id123',
      'https://vk.com/call/join/',
      'not a link',
      'short',
      'хеш-по-русски-длинный-очень',
      'https://vk.com/call/join/with space',
    ]) {
      expect(parseCallHash(input), isNull, reason: '«$input»');
    }
  });

  test('текст ошибки — ровно из спецификации (ист. 15)', () {
    expect(kInvalidCallLink, 'Это не ссылка на звонок VK');
  });

  test('хеш в списке показывается укороченным', () {
    expect(maskHash(hash), 'kw6VzM…QTnUA');
    expect(maskHash('abcdefghijklmnop'), 'abcdef…lmnop');
  });
}
