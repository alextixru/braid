import 'package:flutter/services.dart';

/// Установленное приложение — для режима «по приложениям» (Android).
class InstalledApp {
  const InstalledApp({required this.package, required this.label});

  final String package;
  final String label;
}

/// Канал к сервису VPN на Android (`MethodChannel("org.braid/vpn")`, контракт
/// тикетов 09 ↔ 10). Всё остальное — состояние и команды — идёт через мост
/// ядра: сервис и окно живут в одном процессе.
abstract interface class VpnChannel {
  /// Есть ли сервис: только на Android.
  bool get available;

  /// Системный диалог согласия VPN (из Activity); `true` — согласие есть.
  Future<bool> prepare();

  /// Поднять ядро, **не подключаясь**: окну нужно ядро, чтобы показать
  /// состояние, принять ссылку и правила. Родной метод `initCore`; пока его
  /// нет, единственный путь — [startService], а он подключает сразу
  /// (`ACTION_START` сервиса = `Core::connect`).
  Future<bool> ensureCore();

  /// Запустить сервис и подключить VPN (сервис сам зовёт `Core::connect`);
  /// `true` — запущен.
  Future<bool> startService();

  Future<void> stopService();

  Future<bool> isServiceRunning();

  /// Домен из share-меню, если был; повторно не отдаётся.
  Future<String?> takeSharedDomain();

  /// Ссылка `braid://`, которую ядро не приняло без подтверждения замены хоста.
  Future<String?> takePendingLink();

  /// Установленные приложения; `null` — платформа списка не даёт (ввод вручную).
  Future<List<InstalledApp>?> installedApps();
}

/// Десктоп: сервиса нет.
class NoVpnChannel implements VpnChannel {
  const NoVpnChannel();

  @override
  bool get available => false;

  @override
  Future<bool> prepare() async => true;

  @override
  Future<bool> ensureCore() async => true;

  @override
  Future<bool> startService() async => true;

  @override
  Future<void> stopService() async {}

  @override
  Future<bool> isServiceRunning() async => false;

  @override
  Future<String?> takeSharedDomain() async => null;

  @override
  Future<String?> takePendingLink() async => null;

  @override
  Future<List<InstalledApp>?> installedApps() async => null;
}

/// Android: канал `org.braid/vpn` из `MainActivity`.
class MethodChannelVpn implements VpnChannel {
  const MethodChannelVpn([this._channel = const MethodChannel(name)]);

  static const name = 'org.braid/vpn';
  final MethodChannel _channel;

  @override
  bool get available => true;

  @override
  Future<bool> prepare() async =>
      await _channel.invokeMethod<bool>('prepare') ?? false;

  @override
  Future<bool> ensureCore() async {
    try {
      return await _channel.invokeMethod<bool>('initCore') ?? false;
    } on MissingPluginException {
      return startService();
    } on PlatformException catch (e) {
      if (e.code == 'notImplemented') return startService();
      rethrow;
    }
  }

  @override
  Future<bool> startService() async =>
      await _channel.invokeMethod<bool>('startService') ?? false;

  @override
  Future<void> stopService() => _channel.invokeMethod<void>('stopService');

  @override
  Future<bool> isServiceRunning() async =>
      await _channel.invokeMethod<bool>('isServiceRunning') ?? false;

  @override
  Future<String?> takeSharedDomain() =>
      _channel.invokeMethod<String>('shareDomainPending');

  @override
  Future<String?> takePendingLink() async {
    try {
      return await _channel.invokeMethod<String>('linkPending');
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<List<InstalledApp>?> installedApps() async {
    try {
      final raw = await _channel.invokeListMethod<Map<Object?, Object?>>(
        'installedApps',
      );
      if (raw == null) return null;
      return [
        for (final item in raw)
          if (item['package'] is String)
            InstalledApp(
              package: item['package']! as String,
              label: (item['label'] as String?) ?? item['package']! as String,
            ),
      ];
    } on MissingPluginException {
      return null;
    } on PlatformException catch (e) {
      if (e.code == 'notImplemented') return null;
      rethrow;
    }
  }
}
