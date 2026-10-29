// Ход сценария в браузере (вход VK, капча): страницу опрашивают, пока она не
// отдаст ответ; закрытое окно, истёкшее время и отказ тоже дают ответ — ни
// один путь не оставляет сценарий висеть.

import 'package:braid/services/captcha/web_flow.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';

const fast = Duration(milliseconds: 5);

WebFlow<String> flowOver(
  FakeWebSession session,
  Future<String?> Function() probe, {
  Duration timeout = const Duration(seconds: 5),
}) {
  return WebFlow.poll(
    session: session,
    probe: (_) => probe(),
    onClosed: 'закрыто',
    onTimeout: 'время вышло',
    interval: fast,
    timeout: timeout,
  );
}

void main() {
  test('ответ страницы завершает сценарий и закрывает окно', () async {
    final session = FakeWebSession();
    var calls = 0;
    final flow = flowOver(session, () async => ++calls < 3 ? null : 'готово');

    expect(await flow.result, 'готово');
    expect(calls, 3, reason: 'до ответа страницу опрашивали');
    expect(session.closeCalls, 1);
  });

  test('ошибка опроса — страница ещё грузится, опрос продолжается', () async {
    final session = FakeWebSession();
    var calls = 0;
    final flow = flowOver(session, () async {
      if (++calls < 3) throw StateError('страница ещё грузится');
      return 'готово';
    });
    expect(await flow.result, 'готово');
  });

  test('пользователь закрыл окно — ответ «закрыто»', () async {
    final session = FakeWebSession();
    final flow = flowOver(session, () async => null);
    session.userCloses();
    expect(await flow.result, 'закрыто');
  });

  test('время вышло — ответ «время вышло», окно закрыто', () async {
    final session = FakeWebSession();
    final flow = flowOver(
      session,
      () async => null,
      timeout: const Duration(milliseconds: 60),
    );
    expect(await flow.result, 'время вышло');
    expect(session.closeCalls, 1);
  });

  test('отмена закрывает окно и отвечает «закрыто»', () async {
    final session = FakeWebSession();
    final flow = flowOver(session, () async => null);
    flow.cancel();
    expect(await flow.result, 'закрыто');
    expect(session.closeCalls, 1);
  });

  test('медленный опрос не накладывается сам на себя', () async {
    final session = FakeWebSession();
    var running = 0;
    var overlapped = false;
    var calls = 0;
    final flow = flowOver(session, () async {
      running++;
      if (running > 1) overlapped = true;
      await Future<void>.delayed(const Duration(milliseconds: 25));
      running--;
      return ++calls < 3 ? null : 'готово';
    });
    expect(await flow.result, 'готово');
    expect(overlapped, isFalse);
  });

  test('сценарий, который не начался, отвечает сразу и без страницы', () async {
    final flow = WebFlow<String>.failed('нет браузера');
    expect(flow.session, isNull);
    expect(await flow.result, 'нет браузера');
  });
}
