import 'package:url_launcher/url_launcher.dart';

/// Открывает ссылку в браузере системы. Только `https`: ссылки на сборки
/// приходят из подписки хоста, и открывать что попало по ним нельзя.
abstract interface class UrlOpener {
  /// `true` — ссылка передана системе.
  Future<bool> open(String url);
}

class SystemUrlOpener implements UrlOpener {
  const SystemUrlOpener();

  @override
  Future<bool> open(String url) async {
    final uri = Uri.tryParse(url);
    if (uri == null || uri.scheme != 'https' || uri.host.isEmpty) return false;
    return launchUrl(uri, mode: LaunchMode.externalApplication);
  }
}
