// Меню-бар macOS (ист. 56): значок отражает движок и живость, меню управляет
// подключением, движком и режимом, закрытие окна его скрывает, а не завершает
// приложение. Родной слой подменён записывающей подделкой.

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/tray/tray_service.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

class Bench {
  Bench(this.rig, this.tray, this.icons, this.window, this.service);

  final Rig rig;
  final FakeSystemTray tray;
  final FakeTrayIcons icons;
  final FakeWindowControl window;
  final TrayService service;

  FakeBridge get bridge => rig.bridge;
}

/// Ядро готово, меню-бар запущен.
Future<Bench> startTray(
  WidgetTester tester, {
  CoreState? state,
  List<String>? log,
  FakeBridge? bridge,
  Object? prepareError,
}) async {
  final rig = await pumpApp(
    tester,
    rig: Rig(
      bridge: (bridge ?? FakeBridge())..state = state ?? idleState(),
      autoAnswer: false,
    ),
  );
  final tray = FakeSystemTray();
  final icons = FakeTrayIcons();
  final window = FakeWindowControl(log)..prepareError = prepareError;
  final service = TrayService(
    controller: rig.controller,
    tray: tray,
    icons: icons,
    window: window,
  );
  addTearDown(service.dispose);
  await service.start();
  await tester.pump();
  return Bench(rig, tray, icons, window, service);
}

/// Дать слиянию кадров отработать до конца теста: отложенное обновление не
/// должно пережить его.
Future<void> drain(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(seconds: 1));
}

/// Кадр ядра доходит до контроллера и подождали дольше, чем держит слияние.
Future<void> publish(WidgetTester tester, Bench bench, CoreState next) async {
  bench.bridge.emit(next: next);
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

void main() {
  testWidgets('старт: окно готово, значок и меню на месте', (tester) async {
    final bench = await startTray(tester);

    expect(bench.window.prepared, isTrue);
    expect(bench.tray.shown, isTrue);
    expect(bench.tray.icons, ['B:idle']);
    expect(bench.tray.tooltips, ['Braid · Отключено']);
    expect(bench.tray.item('toggle').label, 'Подключить');
  });

  testWidgets('значок меняется ≤ 2 с после смены состояния', (tester) async {
    final bench = await startTray(tester);

    bench.bridge.emit(next: connectedState());
    await tester.pump();
    await tester.pump(const Duration(seconds: 2));

    expect(bench.tray.icons.last, 'A:ok');
    expect(bench.tray.tooltips.last, 'Braid · Подключено · AWG');
    expect(bench.tray.item('toggle').label, 'Отключить');
  });

  testWidgets('переключение движка — новый глиф', (tester) async {
    final bench = await startTray(tester, state: connectedState());
    expect(bench.tray.icons.last, 'A:ok');

    await publish(
      tester,
      bench,
      idleState(
        session: const SessionConnected(),
        active: 'csqtt',
        engines: const [
          EngineView(id: 'awg', kind: 'awg', state: EngineStatus.dead),
          EngineView(id: 'csqtt', kind: 'csqtt', state: EngineStatus.connected),
        ],
      ),
    );
    expect(bench.tray.icons.last, 'C:ok');
  });

  testWidgets('быстрая череда кадров даёт одно обновление', (tester) async {
    final bench = await startTray(tester);
    final before = bench.tray.icons.length;

    bench.bridge.emit(next: idleState(session: const SessionConnecting()));
    await tester.pump(const Duration(milliseconds: 20));
    bench.bridge.emit(next: connectedState());
    await tester.pump(const Duration(milliseconds: 20));
    await tester.pump(const Duration(seconds: 1));

    expect(bench.tray.icons.length, before + 1);
    expect(bench.tray.icons.last, 'A:ok');
  });

  testWidgets('RTT поменялся — значок, подсказка и меню не трогаются', (
    tester,
  ) async {
    final bench = await startTray(tester, state: connectedState());
    final icons = bench.tray.icons.length;
    final tooltips = bench.tray.tooltips.length;
    final menus = bench.tray.menus.length;

    await publish(
      tester,
      bench,
      idleState(
        session: const SessionConnected(),
        active: 'awg',
        engines: const [
          EngineView(
            id: 'awg',
            kind: 'awg',
            state: EngineStatus.connected,
            health: Health(rttMs: 77, handshakeAgeS: 3),
          ),
          EngineView(
            id: 'csqtt',
            kind: 'csqtt',
            warm: true,
            state: EngineStatus.connected,
          ),
        ],
      ),
    );
    expect(bench.tray.icons, hasLength(icons));
    expect(bench.tray.tooltips, hasLength(tooltips));
    expect(bench.tray.menus, hasLength(menus));
  });

  group('меню', () {
    testWidgets('«Подключить» подключает, «Отключить» отключает', (
      tester,
    ) async {
      final bench = await startTray(tester);
      bench.tray.select('toggle');
      await tester.pump();
      expect(bench.bridge.calls, contains('connect'));
      await drain(tester);

      await publish(tester, bench, connectedState());
      expect(bench.tray.item('toggle').label, 'Отключить');
      bench.tray.select('toggle');
      await tester.pump();
      expect(bench.bridge.calls, contains('disconnect'));
      await drain(tester);
    });

    testWidgets('«Движок»: закрепить и вернуть «Авто»', (tester) async {
      final bench = await startTray(tester);
      bench.tray.select('engine:csqtt');
      await tester.pump();
      bench.tray.select('engine:auto');
      await tester.pump();
      expect(bench.bridge.engineModes, [
        const EngineMode.pinned('csqtt'),
        const EngineMode.auto(),
      ]);
      await drain(tester);
    });

    testWidgets('«Режим» выбирает режим подписки', (tester) async {
      final bench = await startTray(tester);
      bench.tray.select('mode:home');
      await tester.pump();
      expect(bench.bridge.rulesChanges, [const RulesChange.selectMode('home')]);
      await drain(tester);
    });

    testWidgets('«Открыть окно» показывает окно', (tester) async {
      final bench = await startTray(tester);
      bench.tray.select('open');
      await tester.pump();
      expect(bench.window.shows, 1);
    });
  });

  testWidgets('закрытие окна скрывает его, приложение остаётся', (
    tester,
  ) async {
    final bench = await startTray(tester, state: connectedState());
    bench.window.userClosesWindow();
    await tester.pump();

    expect(bench.window.hides, 1);
    expect(bench.window.quits, 0);
    expect(bench.bridge.calls, isNot(contains('disconnect')));
  });

  testWidgets('«Выход»: отключаемся, убираем значок, закрываем приложение', (
    tester,
  ) async {
    final log = <String>[];
    final bench = await startTray(tester, state: connectedState(), log: log);
    bench.tray.select('quit');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(bench.bridge.calls, contains('disconnect'));
    expect(bench.tray.disposed, isTrue);
    expect(bench.window.quits, 1);
  });

  testWidgets('«Выход» без сессии не трогает ядро', (tester) async {
    final bench = await startTray(tester);
    bench.tray.select('quit');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(bench.bridge.calls, isNot(contains('disconnect')));
    expect(bench.window.quits, 1);
  });

  testWidgets(
    'родной слой отказал — приложение живёт, пользователь предупреждён',
    (tester) async {
      final bench = await startTray(tester);
      final notices = bench.rig.notices;

      bench.tray.iconError = StateError('нет значка');
      await publish(tester, bench, connectedState());
      await publish(tester, bench, idleState());

      expect(notices.where((m) => m.contains('меню-баре')), hasLength(1));
      expect(bench.window.quits, 0);
    },
  );

  group('скрытое окно', () {
    testWidgets('вопрос, на который нужен ответ, поднимает окно', (
      tester,
    ) async {
      final bench = await startTray(tester);
      bench.window.userClosesWindow();
      await tester.pump();
      expect(bench.window.shows, 0);

      // Первое подключение просит согласия на правило sudoers: диалог живёт в окне.
      unawaited(bench.rig.controller.addDomainWithConfirmation('site.com'));
      await tester.pump();
      expect(bench.window.shows, 1);

      // Диалог показывает EventHost; отвечаем «Отмена», чтобы тест закончился.
      await tester.pumpAndSettle();
      await tester.tap(find.text('Отмена'));
      await tester.pumpAndSettle();
    });

    testWidgets('сбой подключения из меню показывается в окне', (tester) async {
      final bridge = FakeBridge()
        ..connectError = const BridgeError(
          kind: BridgeErrorKind.conflict,
          message: 'Отключи utun5',
          details: ['utun5'],
        );
      final bench = await startTray(tester, bridge: bridge);
      bench.window.userClosesWindow();
      await tester.pump();

      bench.tray.select('toggle');
      await tester.pump();
      await tester.pump();
      expect(bench.window.shows, 1);
      await drain(tester);
    });

    testWidgets('окно видно — оно не «поднимается» заново', (tester) async {
      final bridge = FakeBridge()
        ..connectError = const BridgeError(
          kind: BridgeErrorKind.conflict,
          message: 'Отключи utun5',
          details: ['utun5'],
        );
      final bench = await startTray(tester, bridge: bridge);
      bench.tray.select('toggle');
      await tester.pump();
      await tester.pump();
      expect(bench.window.shows, 0);
      await drain(tester);
    });

    testWidgets('сообщение не от меню окно не трогает', (tester) async {
      final bench = await startTray(tester);
      bench.window.userClosesWindow();
      await tester.pump();

      bench.rig.controller.showNotice('Подписка обновлена');
      await tester.pump();
      expect(bench.window.shows, 0);
    });
  });

  testWidgets(
    'окно не перехватилось — значок всё равно есть, пользователь предупреждён',
    (tester) async {
      final bench = await startTray(
        tester,
        prepareError: StateError('нет плагина окна'),
      );
      expect(bench.tray.shown, isTrue);
      expect(bench.tray.icons, ['B:idle']);
      expect(
        bench.rig.notices.single,
        'Закрытие окна завершит приложение: перехватить его не удалось',
      );
    },
  );
}
