/// Ход одного сценария в браузере: открытая страница, ответ и отмена.
///
/// Вход VK и капча устроены одинаково: страницу открывают и каждые 400 мс
/// спрашивают, не пора ли забрать ответ (адрес-перенаправление с токеном,
/// `success_token` в перехваченных ответах). Так делает и FOCSQ: у обоих
/// WebView нет надёжного события «страница отдала ответ», а опрос работает
/// одинаково и во встроенном браузере, и в окне ОС.
library;

import 'dart:async';

import 'package:braid/services/captcha/web_session.dart';

class WebFlow<T> {
  WebFlow._(this.session, this.result, this._cancel);

  /// Сценарий, который не удалось начать: ответ готов сразу, страницы нет.
  factory WebFlow.failed(T value) =>
      WebFlow._(null, Future.value(value), () {});

  /// Опрашивает страницу каждые [interval], пока [probe] не вернёт ответ.
  /// Закрытие окна даёт [onClosed], выход за [timeout] — [onTimeout].
  factory WebFlow.poll({
    required WebSession session,
    required Future<T?> Function(WebSession session) probe,
    required T onClosed,
    required T onTimeout,
    Duration interval = const Duration(milliseconds: 400),
    Duration timeout = const Duration(minutes: 2),
  }) {
    final done = Completer<T>();
    Timer? ticker;
    Timer? deadline;
    var probing = false;

    void finish(T value) {
      if (done.isCompleted) return;
      ticker?.cancel();
      deadline?.cancel();
      done.complete(value);
      unawaited(_closeQuietly(session));
    }

    ticker = Timer.periodic(interval, (_) async {
      // Медленный опрос не накладывается на следующий.
      if (done.isCompleted || probing) return;
      probing = true;
      try {
        final answer = await probe(session);
        if (answer != null) finish(answer);
      } on Object {
        // Страница ещё грузится или скрипт не принят — спросим в следующий раз.
      } finally {
        probing = false;
      }
    });
    deadline = Timer(timeout, () => finish(onTimeout));
    unawaited(
      session.closed.then(
        (_) => finish(onClosed),
        onError: (Object _) => finish(onClosed),
      ),
    );
    return WebFlow._(session, done.future, () => finish(onClosed));
  }

  /// Открытая страница; `null` — открыть не удалось.
  final WebSession? session;

  /// Ответ сценария: завершается всегда — ответом, закрытием или временем.
  final Future<T> result;

  final void Function() _cancel;

  /// Пользователь отказался: страница закрывается, ответ — «закрыто».
  void cancel() => _cancel();

  static Future<void> _closeQuietly(WebSession session) async {
    try {
      await session.close();
    } on Object {
      // Окно уже закрыто.
    }
  }
}
