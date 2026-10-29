/// Капча VK по событию ядра (ист. 16).
///
/// Движок просит капчу событием `NeedsCaptcha{url, session}`; в состоянии для
/// экрана это `EngineView.user_action = captcha`, а сам движок — `WaitingUser`
/// (автовыбор не считает это смертью). Здесь решается, какой запрос открывать
/// и куда отправлять ответ; окно с капчей показывает `EngineEventsHost`.
///
/// Каждый запрос ядра (его `session`) открывается один раз: пока пользователь
/// отвечал, ядро могло оставить в состоянии прежнюю капчу, и без этой защиты
/// окно открывалось бы снова и снова.
library;

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/captcha/captcha_solver.dart';
import 'package:braid/services/captcha/web_flow.dart';

/// Одна просьба движка решить капчу.
class CaptchaRequest {
  const CaptchaRequest({
    required this.engineId,
    required this.url,
    required this.session,
  });

  final String engineId;

  /// Страница капчи VK.
  final String url;

  /// Сессия запроса в ядре; по ней запросы различаются.
  final String session;
}

class CaptchaCoordinator {
  CaptchaCoordinator({required this.controller, required this.solver});

  final BraidController controller;
  final CaptchaSolver solver;

  final _opened = <String>{};

  /// Следующий запрос, который ещё не открывали; `null` — капчи нет.
  CaptchaRequest? nextRequest(CoreState state) {
    for (final engine in state.engines) {
      final action = engine.userAction;
      if (action is CaptchaAction && !_opened.contains(action.session)) {
        return CaptchaRequest(
          engineId: engine.id,
          url: action.url,
          session: action.session,
        );
      }
    }
    return null;
  }

  /// Открыть страницу капчи. Запрос с этого момента считается открытым.
  Future<WebFlow<String>> begin(CaptchaRequest request) async {
    _opened.add(request.session);
    final url = Uri.tryParse(request.url);
    if (url == null) return WebFlow.failed(CaptchaSolver.webviewFailed);
    return solver.begin(url);
  }

  /// Отдать результат ядру (`success_token` или `error:*`). `null` — принято;
  /// иначе текст для экрана (например, ответ уже устарел).
  Future<String?> deliver(CaptchaRequest request, String result) async {
    try {
      await controller.userInput(
        request.engineId,
        UserInput.captchaResult(result),
      );
      return null;
    } on BridgeError catch (error) {
      return error.message;
    } on Object catch (error) {
      return BraidController.describeError(error);
    }
  }
}
