// Ввод своих правил (ист. 51, 54): домен и подсеть приводятся к виду, который
// принимает ядро, а буфер обмена даёт домен для «+ Домен». Эталонные случаи —
// те же, что гоняет Rust против ядра (`app/rust/src/contract.rs`).

import 'package:braid/util/rule_input.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fixtures.dart';

void main() {
  final cases = fixtureObject('rule_input_cases.json');

  group('домены', () {
    for (final raw in cases['domains'] as List<dynamic>) {
      final item = raw as Map<String, dynamic>;
      final input = item['input'] as String;
      if (item.containsKey('ok')) {
        test('«$input» → ${item['ok']}', () {
          expect(RuleInput.normalizeDomain(input), item['ok']);
        });
      } else {
        test('«$input» не домен', () {
          expect(
            () => RuleInput.normalizeDomain(input),
            throwsA(isA<RuleInputException>()),
          );
        });
      }
    }

    test('IP вместо домена объясняется отдельно: нужна подсеть', () {
      expect(
        () => RuleInput.normalizeDomain('1.2.3.4'),
        throwsA(
          isA<RuleInputException>().having(
            (e) => e.message,
            'message',
            contains('подсеть'),
          ),
        ),
      );
    });

    test('кириллический домен объясняется отдельно: нужен punycode', () {
      expect(
        () => RuleInput.normalizeDomain('пример.рф'),
        throwsA(
          isA<RuleInputException>().having(
            (e) => e.message,
            'message',
            contains('xn--'),
          ),
        ),
      );
    });

    test('пустой ввод просит ввести домен или подсеть', () {
      expect(
        () => RuleInput.normalizeDomain('  '),
        throwsA(
          isA<RuleInputException>().having(
            (e) => e.message,
            'message',
            'Введи домен или подсеть',
          ),
        ),
      );
    });
  });

  group('подсети', () {
    for (final raw in cases['cidrs'] as List<dynamic>) {
      final item = raw as Map<String, dynamic>;
      final input = item['input'] as String;
      if (item.containsKey('ok')) {
        test('«$input» → ${item['ok']}', () {
          expect(RuleInput.normalizeCidr(input), item['ok']);
        });
      } else {
        test('«$input» не подсеть', () {
          expect(
            () => RuleInput.normalizeCidr(input),
            throwsA(isA<RuleInputException>()),
          );
        });
      }
    }
  });

  group('одно поле для домена и подсети', () {
    test('ссылка становится доменом', () {
      expect(
        RuleInput.classify('https://x.y/путь'),
        const NormalizedRule(RuleKind.domain, 'x.y'),
      );
    });

    test('IPv4 с длиной и без становится подсетью', () {
      expect(
        RuleInput.classify(' 10.1.2.3/8 '),
        const NormalizedRule(RuleKind.subnet, '10.0.0.0/8'),
      );
      expect(
        RuleInput.classify('192.168.1.5'),
        const NormalizedRule(RuleKind.subnet, '192.168.1.5/32'),
      );
    });

    test('сломанная подсеть остаётся ошибкой подсети, а не «домена»', () {
      expect(
        () => RuleInput.classify('10.0.0.0/33'),
        throwsA(
          isA<RuleInputException>().having(
            (e) => e.message,
            'message',
            contains('подсеть'),
          ),
        ),
      );
    });

    test(
      'мусор — ошибка, ввод не теряется (исключение несёт только объяснение)',
      () {
        expect(
          () => RuleInput.classify('привет, мир'),
          throwsA(isA<RuleInputException>()),
        );
        expect(
          () => RuleInput.classify(''),
          throwsA(isA<RuleInputException>()),
        );
      },
    );
  });

  group('домен из буфера обмена', () {
    const expectations = <String, String?>{
      'https://site.com/page?utm=1': 'site.com',
      'https://www.site.com/': 'site.com',
      'site.com': 'site.com',
      'WWW.Site.COM': 'site.com',
      'Смотри https://news.site.org/a/b, интересно': 'news.site.org',
      '(https://example.org/x)': 'example.org',
      '*.google.com': '*.google.com',
      'привет': null,
      '': null,
      '   \n  ': null,
      '1.2.3.4': null,
      'http://localhost:3000': null,
      'braid://eyJ2IjoxLCJ1cmwiOiJodHRwczovL3gifQ': null,
      'e.g.': null,
    };
    expectations.forEach((text, domain) {
      test('«${text.replaceAll('\n', r'\n')}» → ${domain ?? 'нет домена'}', () {
        expect(RuleInput.domainFromText(text), domain);
      });
    });
  });
}
