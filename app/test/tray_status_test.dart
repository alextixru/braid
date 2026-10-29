// Значок и меню в меню-баре macOS (ист. 56, G05): глиф — по движку, цвет — по
// живости, меню — подключение, движок, режим, окно, выход. Здесь — только
// решение «что показать» по состоянию ядра; сам значок рисует ОС.

import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/tray/tray_status.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/states.dart';

const down = EngineView(id: 'awg', kind: 'awg', state: EngineStatus.dead);

CoreState connectedWith(EngineView active, {String id = 'awg'}) => idleState(
  session: const SessionConnected(),
  active: id,
  engines: [active, if (active.id != 'csqtt') csqttEngine],
);

TrayMenuItem item(List<TrayMenuItem> menu, String id) =>
    menu.firstWhere((entry) => entry.id == id, orElse: () => fail('нет «$id»'));

void main() {
  group('глиф и цвет', () {
    test('не подключено: серый общий значок', () {
      final status = trayStatus(idleState());
      expect(status.glyph, 'B');
      expect(status.tone, TrayTone.idle);
      expect(status.headline, 'Отключено');
    });

    test('подключено через AWG: «A» и зелёный', () {
      final status = trayStatus(connectedState());
      expect(status.glyph, 'A');
      expect(status.tone, TrayTone.ok);
      expect(status.headline, 'Подключено · AWG');
    });

    test('подключено через CSQTT: «C»', () {
      final status = trayStatus(
        connectedWith(
          const EngineView(
            id: 'csqtt',
            kind: 'csqtt',
            state: EngineStatus.connected,
          ),
          id: 'csqtt',
        ),
      );
      expect(status.glyph, 'C');
      expect(status.tone, TrayTone.ok);
    });

    test('движок деградировал — жёлтый, мёртв — красный', () {
      const degraded = EngineView(
        id: 'awg',
        kind: 'awg',
        state: EngineStatus.degraded,
      );
      expect(trayStatus(connectedWith(degraded)).tone, TrayTone.warning);
      expect(trayStatus(connectedWith(down)).tone, TrayTone.problem);
    });

    test('движок запускается или ждёт пользователя', () {
      const starting = EngineView(
        id: 'awg',
        kind: 'awg',
        state: EngineStatus.starting,
      );
      const asking = EngineView(
        id: 'awg',
        kind: 'awg',
        state: EngineStatus.waitingUser,
      );
      expect(trayStatus(connectedWith(starting)).tone, TrayTone.working);
      expect(trayStatus(connectedWith(asking)).tone, TrayTone.warning);
    });

    test('подключаюсь: синий, глиф движка, если он уже известен', () {
      final status = trayStatus(
        idleState(session: const SessionConnecting(), active: 'awg'),
      );
      expect(status.tone, TrayTone.working);
      expect(status.headline, 'Подключаюсь…');
      expect(status.glyph, 'A');
    });

    test('кнопка нажата, кадр «подключаюсь» ещё не пришёл', () {
      final status = trayStatus(idleState(), busy: true);
      expect(status.tone, TrayTone.working);
      expect(status.headline, 'Подключаюсь…');
    });

    test('нет связи с хостом — красный; прервано системой — жёлтый', () {
      final noHost = trayStatus(
        idleState(session: const SessionNoHost('awg: нет ответа')),
      );
      expect(noHost.tone, TrayTone.problem);
      expect(noHost.headline, 'Нет связи с хостом');

      final taken = trayStatus(
        idleState(session: const SessionInterrupted(byOtherVpn: true)),
      );
      expect(taken.tone, TrayTone.warning);
      expect(taken.headline, 'VPN перехвачен другим приложением');
    });

    test('у значка один ключ на пару «глиф + цвет»', () {
      expect(
        trayStatus(connectedState()).iconKey,
        trayStatus(connectedState(admin: true)).iconKey,
      );
      expect(
        trayStatus(connectedState()).iconKey,
        isNot(trayStatus(idleState()).iconKey),
      );
    });

    test('подсказка к значку называет приложение и состояние', () {
      expect(trayStatus(connectedState()).tooltip, 'Braid · Подключено · AWG');
    });
  });

  group('меню', () {
    test('порядок: состояние, подключение, движок, режим, окно, выход', () {
      final menu = trayMenu(idleState(), catalogFixture);
      expect(
        [for (final entry in menu) entry.isSeparator ? '—' : entry.id],
        ['status', '—', 'toggle', 'engine', 'mode', '—', 'open', 'quit'],
      );
      expect(item(menu, 'status').enabled, isFalse);
      expect(item(menu, 'status').label, 'Отключено');
      expect(item(menu, 'open').label, 'Открыть окно');
      expect(item(menu, 'quit').label, 'Выход');
    });

    test('подключение: «Подключить» / «Отключить» / «Отменить»', () {
      String label(CoreState state, {bool busy = false}) =>
          item(trayMenu(state, catalogFixture, busy: busy), 'toggle').label;
      expect(label(idleState()), 'Подключить');
      expect(label(connectedState()), 'Отключить');
      expect(label(idleState(session: const SessionConnecting())), 'Отменить');
      expect(label(idleState(), busy: true), 'Отменить');
      expect(
        label(idleState(session: const SessionNoHost('нет ответа'))),
        'Повторить',
      );
    });

    test('без подписки подключаться нечем', () {
      final toggle = item(
        trayMenu(noSubscriptionState, catalogFixture),
        'toggle',
      );
      expect(toggle.enabled, isFalse);
      expect(toggle.label, 'Подключить');
    });

    test('«Движок»: Авто и каждый движок, закреплённый отмечен', () {
      List<TrayMenuItem> children(CoreState state) =>
          item(trayMenu(state, catalogFixture), 'engine').children;

      final auto = children(idleState());
      expect(
        [for (final c in auto) c.id],
        ['engine:auto', 'engine:awg', 'engine:csqtt'],
      );
      expect([for (final c in auto) c.label], ['Авто', 'AWG', 'CSQTT']);
      expect([for (final c in auto) c.checked], [true, false, false]);

      final pinned = children(
        idleState(
          engines: const [
            awgEngine,
            EngineView(id: 'csqtt', kind: 'csqtt', pinned: true),
          ],
        ),
      );
      expect([for (final c in pinned) c.checked], [false, false, true]);
    });

    test('«Режим»: режимы подписки, текущий отмечен', () {
      final children = item(
        trayMenu(idleState(), catalogFixture),
        'mode',
      ).children;
      expect([for (final c in children) c.id], ['mode:work', 'mode:home']);
      expect([for (final c in children) c.label], ['Работа', 'Дом']);
      expect([for (final c in children) c.checked], [true, false]);
    });

    test('режимов нет — пункт «Режим» недоступен; движков нет — «Движок»', () {
      final menu = trayMenu(
        idleState(engines: const [], rules: const RulesView()),
        const RulesCatalog(),
      );
      expect(item(menu, 'mode').enabled, isFalse);
      expect(item(menu, 'engine').enabled, isFalse);
    });

    test('одинаковые состояния дают равные меню — лишних обновлений нет', () {
      expect(
        trayMenu(connectedState(), catalogFixture),
        trayMenu(connectedState(), catalogFixture),
      );
      expect(
        trayMenu(connectedState(), catalogFixture),
        isNot(trayMenu(idleState(), catalogFixture)),
      );
    });
  });
}
