// Капча VK (ист. 16): по `NeedsCaptcha` открывается страница капчи, а ответ —
// `success_token` из перехваченных ответов страницы — уходит ядру в
// `user_input`. Здесь — сама страница и ответ; браузер подменён.

import 'package:braid/services/captcha/captcha_solver.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';

final captchaUrl = Uri.parse(
  'https://id.vk.com/not_robot_captcha?domain=vk.com',
);

CaptchaSolver solverOver(
  FakeWebLauncher launcher, {
  Duration timeout = const Duration(seconds: 5),
}) {
  return CaptchaSolver(
    launcher: launcher,
    interval: const Duration(milliseconds: 5),
    timeout: timeout,
  );
}

/// Страница, перехватившая ответ VK с токеном после решения капчи.
String Function(String) pageCapturing(String body) {
  return (script) => script.contains('__braidHooked') ? 'hooked' : body;
}

void main() {
  test(
    'название и размер окна капчи; ядру отдаётся то, что решила страница',
    () async {
      final launcher = FakeWebLauncher();
      launcher.session.evaluator = pageCapturing(
        '{"response":{"success_token":"tok_ABC-123"}}',
      );
      final flow = await solverOver(launcher).begin(captchaUrl);

      expect(await flow.result, 'tok_ABC-123');
      expect(launcher.requests.single.url, captchaUrl);
      expect(launcher.requests.single.title, 'Подтвердите, что вы не робот');
      expect(launcher.session.closeCalls, 1);
    },
  );

  test(
    'токен в ответе, пришедшем строкой внутри строки, тоже находится',
    () async {
      final launcher = FakeWebLauncher();
      launcher.session.evaluator = pageCapturing(
        r'{"data":"{\"success_token\":\"escaped_9-Z\"}"}',
      );
      final flow = await solverOver(launcher).begin(captchaUrl);
      expect(await flow.result, 'escaped_9-Z');
    },
  );

  test('пока токена нет, перехватчик ставится заново и опрос идёт', () async {
    final launcher = FakeWebLauncher();
    var reads = 0;
    launcher.session.evaluator = (script) {
      if (script.contains('__braidHooked')) return 'hooked';
      return ++reads < 3 ? '{"status":"pending"}' : '{"success_token":"late"}';
    };
    final flow = await solverOver(launcher).begin(captchaUrl);
    expect(await flow.result, 'late');
    expect(
      launcher.session.scripts.where((s) => s.contains('__braidHooked')),
      hasLength(3),
      reason: 'перехватчик ставится на каждый опрос — страница могла перезагрузиться',
    );
  });

  test('окно закрыли — ядру уходит error:closed', () async {
    final launcher = FakeWebLauncher();
    final flow = await solverOver(launcher).begin(captchaUrl);
    launcher.session.userCloses();
    expect(await flow.result, 'error:closed');
  });

  test('отказ в диалоге — error:closed, окно закрыто', () async {
    final launcher = FakeWebLauncher();
    final flow = await solverOver(launcher).begin(captchaUrl);
    flow.cancel();
    expect(await flow.result, 'error:closed');
    expect(launcher.session.closeCalls, 1);
  });

  test('время вышло — error:timeout', () async {
    final launcher = FakeWebLauncher();
    final flow = await solverOver(
      launcher,
      timeout: const Duration(milliseconds: 60),
    ).begin(captchaUrl);
    expect(await flow.result, 'error:timeout');
  });

  test('не https — страницу не открываем: error:webview', () async {
    final launcher = FakeWebLauncher();
    final flow = await solverOver(launcher)
        .begin(Uri.parse('http://id.vk.com/not_robot_captcha'));
    expect(await flow.result, 'error:webview');
    expect(launcher.requests, isEmpty);
  });

  test('браузера нет или окно не открылось — error:webview', () async {
    final none = FakeWebLauncher(available: false);
    expect(
      await (await solverOver(none).begin(captchaUrl)).result,
      'error:webview',
    );

    final broken = FakeWebLauncher()..openError = StateError('нет окна');
    expect(
      await (await solverOver(broken).begin(captchaUrl)).result,
      'error:webview',
    );
  });
}
