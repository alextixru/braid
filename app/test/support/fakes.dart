/// Подделки сервисов приложения: канал сервиса Android, deeplink, буфер
/// обмена, файловые диалоги, каталоги.
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:braid/services/app_dirs.dart';
import 'package:braid/services/clipboard_access.dart';
import 'package:braid/services/deep_links.dart';
import 'package:braid/services/qr_scanner.dart';
import 'package:braid/services/rules_files.dart';
import 'package:braid/services/url_opener.dart';
import 'package:braid/services/vpn_channel.dart';

class FakeVpn implements VpnChannel {
  FakeVpn({
    this.consent = true,
    this.onEnsureCore,
    this.onStartService,
    this.apps,
  });

  /// Что ответит системный диалог согласия VPN.
  bool consent;

  /// Сервис построил ядро (`initCore` или первый запуск).
  void Function()? onEnsureCore;

  /// Сервис получил `ACTION_START`: он сам зовёт `Core::connect` — тест
  /// подставляет то, что сделает сервис (подключит, остановится с ошибкой).
  void Function()? onStartService;

  /// Список установленных приложений; `null` — платформа списка не даёт.
  List<InstalledApp>? apps;

  String? sharedDomain;
  String? pendingLink;

  /// Сервис жив (привязан к ядру).
  bool running = false;
  final calls = <String>[];

  @override
  bool get available => true;

  @override
  Future<bool> prepare() async {
    calls.add('prepare');
    return consent;
  }

  @override
  Future<bool> ensureCore() async {
    calls.add('ensureCore');
    onEnsureCore?.call();
    return true;
  }

  @override
  Future<bool> startService() async {
    calls.add('startService');
    running = true;
    onStartService?.call();
    return true;
  }

  @override
  Future<void> stopService() async {
    calls.add('stopService');
    running = false;
  }

  @override
  Future<bool> isServiceRunning() async {
    calls.add('isServiceRunning');
    return running;
  }

  @override
  Future<String?> takeSharedDomain() async {
    calls.add('takeSharedDomain');
    final domain = sharedDomain;
    sharedDomain = null;
    return domain;
  }

  @override
  Future<String?> takePendingLink() async {
    calls.add('takePendingLink');
    final link = pendingLink;
    pendingLink = null;
    return link;
  }

  @override
  Future<List<InstalledApp>?> installedApps() async => apps;
}

class FakeDeepLinks implements DeepLinks {
  FakeDeepLinks({this.initial});

  String? initial;

  /// Родной слой ссылок сломался при запросе ссылки запуска.
  Object? initialError;
  final _controller = StreamController<String>.broadcast();

  void open(String link) => _controller.add(link);

  @override
  Future<String?> initialLink() async {
    final error = initialError;
    if (error != null) throw error;
    return initial;
  }

  @override
  Stream<String> get links => _controller.stream;
}

class FakeClipboard implements ClipboardAccess {
  FakeClipboard([this.text]);

  String? text;

  @override
  Future<String?> readText() async => text;

  @override
  Future<void> writeText(String value) async => text = value;
}

class FakeRulesFiles implements RulesFiles {
  FakeRulesFiles({this.available = true});

  @override
  final bool available;

  /// Что «выбрал» пользователь в диалоге открытия.
  String? toOpen;

  /// Что записали в диалоге сохранения: имя и содержимое.
  final saved = <(String, String)>[];
  String? savePath = '/tmp/braid-rules.json';

  @override
  Future<String?> save(String suggestedName, String content) async {
    saved.add((suggestedName, content));
    return savePath;
  }

  @override
  Future<String?> open() async => toOpen;
}

class FakeDirs implements AppDirs {
  const FakeDirs();

  @override
  Future<String> coreDir() async => '/tmp/braid-test-core';
}

class FakeQrScanner implements QrScanner {
  FakeQrScanner([this.code]);

  /// Что «увидела» камера; `null` — пользователь закрыл сканер.
  String? code;
  int opened = 0;

  @override
  Future<String?> scan(BuildContext context) async {
    opened++;
    return code;
  }
}

class FakeUrlOpener implements UrlOpener {
  final opened = <String>[];
  bool succeeds = true;

  @override
  Future<bool> open(String url) async {
    opened.add(url);
    return succeeds;
  }
}
