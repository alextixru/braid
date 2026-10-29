// Диалог капчи (ист. 16): ядро просит капчу событием `NeedsCaptcha`, пока она
// не решена, движок «ждёт капчу», а автовыбор не считает это смертью. Диалог
// открывается поверх любого экрана; результат уходит ядру в `user_input`.

import 'dart:async';

import 'package:braid/app_extensions.dart';
import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_events_host.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/home/home_screen.dart';
import 'package:braid/services/captcha/captcha_solver.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

const pageUrl = 'https://id.vk.com/not_robot_captcha?domain=vk.com';
const dialog = Key('captcha-dialog');

CoreState asking(String session) => idleState(
  session: const SessionConnected(),
  active: 'awg',
  engines: [
    const EngineView(id: 'awg', kind: 'awg', state: EngineStatus.connected),
    EngineView(
      id: 'csqtt',
      kind: 'csqtt',
      state: EngineStatus.waitingUser,
      userAction: CaptchaAction(url: pageUrl, session: session),
    ),
  ],
);

/// Ядро, которое может отказать в ответе на капчу.
class RefusingBridge extends FakeBridge {
  BridgeError? error;

  @override
  Future<void> userInput(String engine, UserInput input) async {
    final failure = error;
    if (failure != null) throw failure;
    return super.userInput(engine, input);
  }
}

class Bench {
  Bench(this.rig, this.launcher, this.bridge);

  final Rig rig;
  final FakeWebLauncher launcher;
  final RefusingBridge bridge;

  FakeWebSession get page => launcher.session;
}

Future<Bench> openApp(
  WidgetTester tester, {
  FakeWebLauncher? launcher,
  Widget? embedded,
  CoreState? state,
  bool preStart = false,
}) async {
  final bridge = RefusingBridge()..state = state ?? connectedState();
  final rig = Rig(bridge: bridge, autoAnswer: false);
  final web =
      launcher ??
      FakeWebLauncher(session: FakeWebSession(embeddedWidget: embedded));
  EngineServices.install(
    EngineServices(
      controller: rig.controller,
      web: web,
      store: MemoryVkStore(),
      statsFiles: FakeStatsFiles(),
      solver: CaptchaSolver(launcher: web),
    ),
  );
  // Ядро уже запущено и знает о капче, когда в дерево встаёт надстройка.
  if (preStart) {
    unawaited(rig.start());
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 10));
  }
  await pumpApp(
    tester,
    rig: rig,
    extensions: const AppExtensions(wrapApp: wrapWithEngineEvents),
    // Открытый диалог не даёт тишины: ждать её нельзя.
    settle: !preStart,
  );
  return Bench(rig, web, bridge);
}

/// Страница капчи отвечает токеном, как после решения.
void solveOnPage(FakeWebSession page, String token) {
  page.evaluator = (script) => script.contains('__braidHooked')
      ? 'hooked'
      : '{"success_token":"$token"}';
}

Future<void> settleWindow(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 500));
  await tester.pump(const Duration(milliseconds: 500));
}

void main() {
  testWidgets('событие капчи открывает диалог поверх главного экрана', (
    tester,
  ) async {
    final bench = await openApp(tester);
    expect(find.byKey(dialog), findsNothing);

    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();

    expect(find.byType(HomeScreen), findsOneWidget);
    expect(find.byKey(dialog), findsOneWidget);
    expect(find.text('Подтвердите, что вы не робот'), findsOneWidget);
    expect(bench.launcher.requests.single.url, Uri.parse(pageUrl));

    bench.page.userCloses();
    await settleWindow(tester);
  });

  testWidgets('капча уже ждёт при запуске — диалог открывается сам', (
    tester,
  ) async {
    final bench = await openApp(tester, state: asking('s0'), preStart: true);
    await tester.pump();
    await tester.pump();

    expect(find.byKey(dialog), findsOneWidget);
    expect(bench.launcher.requests.single.url, Uri.parse(pageUrl));

    bench.page.userCloses();
    await settleWindow(tester);
  });

  testWidgets('капча решена — диалог закрыт, токен ушёл ядру', (tester) async {
    final bench = await openApp(tester);
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    expect(find.byKey(dialog), findsOneWidget);

    solveOnPage(bench.page, 'tok_ABC-123');
    await settleWindow(tester);

    expect(find.byKey(dialog), findsNothing);
    expect(bench.rig.bridge.userInputs, [
      ('csqtt', const UserInput.captchaResult('tok_ABC-123')),
    ]);
  });

  testWidgets('«Отмена» отвечает ядру error:closed и закрывает диалог', (
    tester,
  ) async {
    final bench = await openApp(tester);
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();

    await tester.tap(find.byKey(const Key('web-flow-cancel')));
    await settleWindow(tester);

    expect(find.byKey(dialog), findsNothing);
    expect(bench.rig.bridge.userInputs, [
      ('csqtt', const UserInput.captchaResult('error:closed')),
    ]);
    expect(bench.page.closeCalls, greaterThanOrEqualTo(1));
  });

  testWidgets('«Назад» на Android — тот же отказ, а не тихое закрытие', (
    tester,
  ) async {
    final bench = await openApp(tester);
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    expect(find.byKey(dialog), findsOneWidget);

    await tester.binding.handlePopRoute();
    await settleWindow(tester);

    expect(find.byKey(dialog), findsNothing);
    expect(bench.rig.bridge.userInputs, [
      ('csqtt', const UserInput.captchaResult('error:closed')),
    ]);
  });

  testWidgets('тот же запрос ядра второй раз диалог не открывает', (
    tester,
  ) async {
    final bench = await openApp(tester);
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('web-flow-cancel')));
    await settleWindow(tester);
    expect(bench.launcher.requests, hasLength(1));

    // Ядро пока не сняло прежний запрос и публикует кадры дальше.
    bench.rig.bridge.emit(next: asking('s1'));
    await settleWindow(tester);
    expect(find.byKey(dialog), findsNothing);
    expect(bench.launcher.requests, hasLength(1));
  });

  testWidgets('новая капча (другая сессия) открывает диалог снова', (
    tester,
  ) async {
    final bench = await openApp(tester);
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('web-flow-cancel')));
    await settleWindow(tester);

    bench.launcher.session = FakeWebSession();
    bench.rig.bridge.emit(next: asking('s2'));
    await tester.pump();
    await tester.pump();
    expect(find.byKey(dialog), findsOneWidget);
    expect(bench.launcher.requests, hasLength(2));

    bench.launcher.session.userCloses();
    await settleWindow(tester);
  });

  testWidgets('встроенный браузер (Android): страница внутри диалога', (
    tester,
  ) async {
    final bench = await openApp(
      tester,
      embedded: const SizedBox(
        key: Key('embedded-page'),
        width: 50,
        height: 50,
      ),
    );
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();

    expect(find.byKey(const Key('embedded-page')), findsOneWidget);
    expect(find.byKey(dialog), findsOneWidget);

    bench.page.userCloses();
    await settleWindow(tester);
  });

  testWidgets('ядро не приняло ответ (устарел) — сообщение внизу экрана', (
    tester,
  ) async {
    final bench = await openApp(tester);
    bench.bridge.error = const BridgeError(
      kind: BridgeErrorKind.failed,
      message: 'ядро не ждёт капчу или ответ устарел',
      details: [],
    );
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    solveOnPage(bench.page, 'late-token');
    await settleWindow(tester);

    expect(find.text('ядро не ждёт капчу или ответ устарел'), findsOneWidget);
    expect(find.byKey(dialog), findsNothing);
    await tester.pump(const Duration(seconds: 5));
  });

  testWidgets('браузера нет — диалога нет, ядру уходит error:webview', (
    tester,
  ) async {
    final bench = await openApp(
      tester,
      launcher: FakeWebLauncher(available: false),
    );
    bench.rig.bridge.emit(next: asking('s1'));
    await tester.pump();
    await tester.pump();
    await tester.pump();

    expect(find.byKey(dialog), findsNothing);
    expect(bench.rig.bridge.userInputs, [
      ('csqtt', const UserInput.captchaResult('error:webview')),
    ]);
  });
}
