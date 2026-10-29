// Тексты состояния: длительность подключения, дата «не обновлялась с…»,
// название сети и движка. Всё по-русски и без привязки к часовому поясу
// машины: функции получают уже местное время.

import 'package:braid/models/core_state.dart';
import 'package:braid/util/format.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('длительность подключения', () {
    test('минуты, часы и дни пишутся коротко', () {
      expect(formatDuration(const Duration(seconds: 20)), 'только что');
      expect(formatDuration(const Duration(minutes: 1)), '1 мин');
      expect(
        formatDuration(const Duration(minutes: 12, seconds: 50)),
        '12 мин',
      );
      expect(formatDuration(const Duration(hours: 1)), '1 ч');
      expect(
        formatDuration(const Duration(hours: 1, minutes: 5)),
        '1 ч 05 мин',
      );
      expect(
        formatDuration(const Duration(hours: 23, minutes: 59)),
        '23 ч 59 мин',
      );
      expect(formatDuration(const Duration(days: 2)), '2 д');
      expect(
        formatDuration(const Duration(days: 2, hours: 3, minutes: 40)),
        '2 д 3 ч',
      );
    });

    test('отрицательная длительность (часы ушли назад) — «только что»', () {
      expect(formatDuration(const Duration(minutes: -5)), 'только что');
    });
  });

  group('дата и время', () {
    final now = DateTime(2026, 10, 29, 15, 30);

    test('сегодня и вчера называются словами, время с ведущим нулём', () {
      expect(
        formatWhen(DateTime(2026, 10, 29, 9, 5), now: now),
        'сегодня, 09:05',
      );
      expect(formatWhen(DateTime(2026, 10, 28, 8), now: now), 'вчера, 08:00');
    });

    test('в этом году — число и месяц, в прошлом — ещё и год', () {
      expect(
        formatWhen(DateTime(2026, 10, 3, 23, 59), now: now),
        '3 окт, 23:59',
      );
      expect(
        formatWhen(DateTime(2026, 1, 12, 7, 0), now: now),
        '12 янв, 07:00',
      );
      expect(formatWhen(DateTime(2025, 12, 31, 7, 0), now: now), '31 дек 2025');
    });

    test('вчера считается по календарю, а не по 24 часам', () {
      final earlyMorning = DateTime(2026, 10, 29, 0, 10);
      expect(
        formatWhen(DateTime(2026, 10, 28, 23, 50), now: earlyMorning),
        'вчера, 23:50',
      );
    });

    test('все двенадцать месяцев названы', () {
      const months = [
        'янв',
        'фев',
        'мар',
        'апр',
        'мая',
        'июн',
        'июл',
        'авг',
        'сен',
        'окт',
        'ноя',
        'дек',
      ];
      for (var m = 1; m <= 12; m++) {
        // 20-е число не бывает ни «сегодня», ни «вчера» при now = 29 октября.
        final text = formatWhen(
          DateTime(2026, m, 20, 12),
          now: DateTime(2026, 10, 29),
        );
        expect(text, '20 ${months[m - 1]}, 12:00');
      }
    });
  });

  group('сеть', () {
    test('Wi-Fi и сеть по шлюзу различаются видом и хвостом хеша', () {
      expect(networkLabel('wifi:00aa11bb22cc33dd'), 'Wi‑Fi · 33dd');
      expect(networkLabel('gw:99887766aabbccdd'), 'Сеть · ccdd');
      expect(networkLabel('00aa11bb22cc33dd'), 'Сеть · 33dd');
      expect(networkLabel('x'), 'Сеть · x');
    });
  });

  group('движки', () {
    const awg = EngineView(id: 'awg', kind: 'awg');
    const csqtt = EngineView(id: 'csqtt', kind: 'csqtt');
    const slot = EngineView(id: 'wg-next', kind: 'wg-next');

    test('известные виды — заглавными, чужой слот — как есть заглавными', () {
      final all = [awg, csqtt, slot];
      expect(engineLabel(awg, all), 'AWG');
      expect(engineLabel(csqtt, all), 'CSQTT');
      expect(engineLabel(slot, all), 'WG-NEXT');
    });

    test('два движка одного вида различаются идентификатором', () {
      const second = EngineView(id: 'awg-2', kind: 'awg');
      expect(engineLabel(awg, [awg, second]), 'AWG · awg');
      expect(engineLabel(second, [awg, second]), 'AWG · awg-2');
    });

    test('причина из ядра называет движок по-человечески', () {
      final all = [awg, csqtt];
      expect(
        prettyReason('awg недоступен, закреплён вручную', all),
        'AWG недоступен, закреплён вручную',
      );
      expect(prettyReason('csqtt: нет ответа', all), 'CSQTT: нет ответа');
      expect(
        prettyReason('Проба к хосту не ответила', all),
        'Проба к хосту не ответила',
      );
      expect(prettyReason('awgx что-то', all), 'awgx что-то');
    });
  });

  group('склонение по числу', () {
    test('домен, домена, доменов', () {
      String domains(int n) =>
          '$n ${ruPlural(n, 'домен', 'домена', 'доменов')}';
      expect(domains(0), '0 доменов');
      expect(domains(1), '1 домен');
      expect(domains(2), '2 домена');
      expect(domains(4), '4 домена');
      expect(domains(5), '5 доменов');
      expect(domains(11), '11 доменов');
      expect(domains(12), '12 доменов');
      expect(domains(14), '14 доменов');
      expect(domains(21), '21 домен');
      expect(domains(22), '22 домена');
      expect(domains(111), '111 доменов');
      expect(domains(101), '101 домен');
    });
  });
}
