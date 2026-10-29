// «Войти через VK» (ист. 78): WebView OAuth по образцу FOCSQ — implicit-вход,
// токен приходит в адресе возврата `…/blank.html#access_token=…`. Браузер
// подменён страницей из теста; ни один тест не ходит в VK.

import 'package:braid/services/vk/vk_oauth.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';

const redirectWithToken =
    'https://oauth.vk.ru/blank.html#access_token=synthetic-token&expires_in=0&user_id=1';

VkOAuth oauthOver(
  FakeWebLauncher launcher, {
  Duration timeout = const Duration(seconds: 5),
}) {
  return VkOAuth(
    launcher: launcher,
    interval: const Duration(milliseconds: 5),
    timeout: timeout,
  );
}

void main() {
  test('адрес входа: implicit-вход VK с возвратом на blank.html', () {
    final url = const VkOAuthConfig().authorizeUrl;
    expect(url.scheme, 'https');
    expect(url.host, 'oauth.vk.ru');
    expect(url.path, '/authorize');
    expect(url.queryParameters, {
      'client_id': '7793118',
      'scope': '1073737727',
      'redirect_uri': 'https://oauth.vk.ru/blank.html',
      'display': 'page',
      'response_type': 'token',
      'revoke': '1',
      'v': '5.199',
    });
  });

  test('окно открывается на адресе входа', () async {
    final launcher = FakeWebLauncher();
    final flow = await oauthOver(launcher).begin();
    expect(launcher.requests, hasLength(1));
    expect(launcher.requests.single.url, const VkOAuthConfig().authorizeUrl);
    expect(launcher.requests.single.title, 'Вход через VK');
    flow.cancel();
    await flow.result;
  });

  test('токен берётся из адреса возврата, окно закрывается', () async {
    final launcher = FakeWebLauncher();
    final flow = await oauthOver(launcher).begin();
    launcher.session.url = 'https://oauth.vk.ru/authorize?client_id=7793118';
    await Future<void>.delayed(const Duration(milliseconds: 30));
    launcher.session.url = redirectWithToken;

    final result = await flow.result;
    expect(result, isA<VkLoginSucceeded>());
    expect((result as VkLoginSucceeded).token, 'synthetic-token');
    expect(launcher.session.closeCalls, 1);
  });

  test('чужой адрес с таким же хвостом токен не отдаёт', () async {
    final launcher = FakeWebLauncher();
    final flow = await oauthOver(
      launcher,
      timeout: const Duration(milliseconds: 80),
    ).begin();
    launcher.session.url =
        'https://evil.example/oauth.vk.ru/blank.html#access_token=stolen';
    final result = await flow.result;
    expect(result, isA<VkLoginFailed>(), reason: 'ждали до конца времени');
    expect((result as VkLoginFailed).message, contains('Время'));
  });

  test('возврат без токена — отказ, текст без содержимого адреса', () async {
    final launcher = FakeWebLauncher();
    final flow = await oauthOver(launcher).begin();
    launcher.session.url = 'https://oauth.vk.ru/blank.html#error=access_denied&error_description=secret-details';
    final result = await flow.result;
    expect(result, isA<VkLoginFailed>());
    final message = (result as VkLoginFailed).message;
    expect(message, 'VK не выдал токен — вход отклонён');
    expect(message, isNot(contains('secret-details')));
  });

  test('окно закрыто до конца входа', () async {
    final launcher = FakeWebLauncher();
    final flow = await oauthOver(launcher).begin();
    launcher.session.userCloses();
    final result = await flow.result;
    expect(
      (result as VkLoginFailed).message,
      'Окно закрыто до завершения входа',
    );
  });

  test('браузера на машине нет — отказ сразу, окно не открывалось', () async {
    final launcher = FakeWebLauncher(available: false);
    final flow = await oauthOver(launcher).begin();
    expect(flow.session, isNull);
    expect(
      (await flow.result as VkLoginFailed).message,
      'WebView недоступен на этой машине',
    );
    expect(launcher.requests, isEmpty);
  });

  test('окно не открылось — отказ с понятным текстом', () async {
    final launcher = FakeWebLauncher()..openError = StateError('нет окна');
    final flow = await oauthOver(launcher).begin();
    expect(
      (await flow.result as VkLoginFailed).message,
      'Не удалось открыть окно входа',
    );
  });
}
