/// Вход через VK (ист. 78): WebView OAuth по образцу FOCSQ `vk/token.dart` —
/// implicit-вход, токен приходит в адресе возврата `…/blank.html#access_token=`.
///
/// Здесь только сценарий входа. Токен сохраняет и отдаёт движку
/// `CsqttCredentials`; в журнал и тексты ошибок токен не попадает.
library;

import 'package:braid/services/captcha/web_flow.dart';
import 'package:braid/services/captcha/web_session.dart';

/// Параметры приложения VK.
///
/// Приложение VK — то же, под которым FOCSQ создаёт звонки для ядра CSQTT
/// (идентификатор открытый, он виден в адресе входа). Свой идентификатор
/// подставляется здесь, больше нигде.
class VkOAuthConfig {
  const VkOAuthConfig({
    this.clientId = '7793118',
    this.scope = '1073737727',
    this.apiVersion = '5.199',
  });

  final String clientId;
  final String scope;
  final String apiVersion;

  static const _host = 'oauth.vk.ru';
  static const _returnPath = '/blank.html';

  Uri get authorizeUrl => Uri.https(_host, '/authorize', {
    'client_id': clientId,
    'scope': scope,
    'redirect_uri': 'https://$_host$_returnPath',
    'display': 'page',
    'response_type': 'token',
    'revoke': '1',
    'v': apiVersion,
  });

  /// Это адрес возврата VK: только он несёт токен. Чужой сайт с похожим
  /// хвостом в пути токена не получает.
  bool isReturn(Uri uri) =>
      uri.scheme == 'https' &&
      (uri.host == _host || uri.host == 'oauth.vk.com') &&
      uri.path == _returnPath;
}

sealed class VkLoginResult {
  const VkLoginResult();
}

final class VkLoginSucceeded extends VkLoginResult {
  const VkLoginSucceeded(this.token);

  final String token;
}

final class VkLoginFailed extends VkLoginResult {
  const VkLoginFailed(this.message);

  /// Текст для экрана, по-русски.
  final String message;
}

class VkOAuth {
  VkOAuth({
    required this.launcher,
    this.config = const VkOAuthConfig(),
    this.interval = const Duration(milliseconds: 500),
    this.timeout = const Duration(minutes: 5),
  });

  final WebLauncher launcher;
  final VkOAuthConfig config;

  /// Как часто смотреть на адрес страницы.
  final Duration interval;

  /// Сколько ждать, пока пользователь войдёт.
  final Duration timeout;

  /// Открыть окно входа и ждать токена. Не бросает: любой сбой — результат
  /// [VkLoginFailed] с текстом для экрана.
  Future<WebFlow<VkLoginResult>> begin() async {
    try {
      if (!await launcher.isAvailable()) {
        return WebFlow.failed(
          const VkLoginFailed('WebView недоступен на этой машине'),
        );
      }
      final session = await launcher.open(
        WebRequest(url: config.authorizeUrl, title: 'Вход через VK'),
      );
      return WebFlow.poll(
        session: session,
        probe: _probe,
        onClosed: const VkLoginFailed('Окно закрыто до завершения входа'),
        onTimeout: const VkLoginFailed(
          'Время ожидания входа истекло — попробуй ещё раз',
        ),
        interval: interval,
        timeout: timeout,
      );
    } on Object {
      return WebFlow.failed(
        const VkLoginFailed('Не удалось открыть окно входа'),
      );
    }
  }

  Future<VkLoginResult?> _probe(WebSession session) async {
    final url = await session.currentUrl();
    final uri = url == null ? null : Uri.tryParse(url);
    if (uri == null || !config.isReturn(uri)) return null;
    final token = _tokenFrom(uri);
    if (token != null) return VkLoginSucceeded(token);
    return const VkLoginFailed('VK не выдал токен — вход отклонён');
  }

  /// Токен приходит во фрагменте; запасной путь — строка запроса.
  static String? _tokenFrom(Uri uri) {
    for (final source in [uri.fragment, uri.query]) {
      if (source.isEmpty) continue;
      try {
        final token = Uri.splitQueryString(source)['access_token'];
        if (token != null && token.isNotEmpty) return token;
      } on FormatException {
        continue;
      }
    }
    return null;
  }
}
