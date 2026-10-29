// Капча по событию ядра (ист. 16): `NeedsCaptcha{url, session}` приходит как
// `EngineView.user_action = captcha`. Каждый запрос ядра открывается один раз,
// ответ уходит в `user_input` того движка, который спросил.

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/captcha/captcha_coordinator.dart';
import 'package:braid/services/captcha/captcha_solver.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/rig.dart';
import 'support/states.dart';

const pageUrl = 'https://id.vk.com/not_robot_captcha?domain=vk.com';

CoreState askingCaptcha({String session = 'session-1', String url = pageUrl}) =>
    idleState(
      session: const SessionConnected(),
      active: 'awg',
      engines: [
        const EngineView(id: 'awg', kind: 'awg', state: EngineStatus.connected),
        EngineView(
          id: 'csqtt',
          kind: 'csqtt',
          state: EngineStatus.waitingUser,
          userAction: CaptchaAction(url: url, session: session),
        ),
      ],
    );

class Refusing extends FakeBridge {
  BridgeError? error;

  @override
  Future<void> userInput(String engine, UserInput input) async {
    final failure = error;
    if (failure != null) throw failure;
    return super.userInput(engine, input);
  }
}

Future<(CaptchaCoordinator, Rig, FakeWebLauncher)> coordinatorOver(
  CoreState state, {
  Refusing? bridge,
}) async {
  final rig = Rig(
    bridge: (bridge ?? Refusing())..state = state,
    autoAnswer: false,
  );
  await rig.start();
  final launcher = FakeWebLauncher();
  final coordinator = CaptchaCoordinator(
    controller: rig.controller,
    solver: CaptchaSolver(
      launcher: launcher,
      interval: const Duration(milliseconds: 5),
      timeout: const Duration(seconds: 5),
    ),
  );
  addTearDown(rig.dispose);
  return (coordinator, rig, launcher);
}

void main() {
  test('запрос берётся у движка, который ждёт капчу', () async {
    final (coordinator, rig, _) = await coordinatorOver(askingCaptcha());
    final request = coordinator.nextRequest(rig.controller.state);
    expect(request, isNotNull);
    expect(request!.engineId, 'csqtt');
    expect(request.url, pageUrl);
    expect(request.session, 'session-1');
  });

  test('капчи нет — запроса нет', () async {
    final (coordinator, rig, _) = await coordinatorOver(connectedState());
    expect(coordinator.nextRequest(rig.controller.state), isNull);
  });

  test(
    'открытый запрос второй раз не выдаётся, новая сессия — выдаётся',
    () async {
      final (coordinator, rig, _) = await coordinatorOver(askingCaptcha());
      final first = coordinator.nextRequest(rig.controller.state)!;
      final flow = await coordinator.begin(first);
      flow.cancel();
      await flow.result;

      expect(coordinator.nextRequest(rig.controller.state), isNull);

      rig.bridge.emit(next: askingCaptcha(session: 'session-2'));
      await Future<void>.delayed(Duration.zero);
      expect(
        coordinator.nextRequest(rig.controller.state)?.session,
        'session-2',
      );
    },
  );

  test('begin открывает страницу капчи из запроса ядра', () async {
    final (coordinator, rig, launcher) = await coordinatorOver(askingCaptcha());
    final request = coordinator.nextRequest(rig.controller.state)!;
    final flow = await coordinator.begin(request);
    expect(launcher.requests.single.url, Uri.parse(pageUrl));
    flow.cancel();
    await flow.result;
  });

  test(
    'адрес капчи нечитаем — ядру уйдёт error:webview, окно не открывалось',
    () async {
      final (coordinator, rig, launcher) = await coordinatorOver(
        askingCaptcha(url: 'http://[bad'),
      );
      final request = coordinator.nextRequest(rig.controller.state)!;
      final flow = await coordinator.begin(request);
      expect(await flow.result, CaptchaSolver.webviewFailed);
      expect(launcher.requests, isEmpty);
    },
  );

  test('ответ уходит ядру в user_input того движка, что спросил', () async {
    final (coordinator, rig, _) = await coordinatorOver(askingCaptcha());
    final request = coordinator.nextRequest(rig.controller.state)!;
    expect(await coordinator.deliver(request, 'tok_ABC-123'), isNull);
    expect(rig.bridge.userInputs, [
      ('csqtt', const UserInput.captchaResult('tok_ABC-123')),
    ]);
  });

  test('ядро не приняло ответ (устарел) — текст для экрана', () async {
    final bridge = Refusing()
      ..error = const BridgeError(
        kind: BridgeErrorKind.failed,
        message: 'ядро не ждёт капчу или ответ устарел',
        details: [],
      );
    final (coordinator, rig, _) = await coordinatorOver(
      askingCaptcha(),
      bridge: bridge,
    );
    final request = coordinator.nextRequest(rig.controller.state)!;
    expect(
      await coordinator.deliver(request, 'tok'),
      'ядро не ждёт капчу или ответ устарел',
    );
  });
}
