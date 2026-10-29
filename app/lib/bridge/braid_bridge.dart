/// Всё, что приложение берёт у ядра. Единственный шов между Dart и Rust:
/// экраны и контроллер знают только этот интерфейс; настоящая реализация —
/// `RustBridge` (flutter_rust_bridge), в тестах — подделка.
///
/// Ошибки команд приходят как [BridgeError] (вид + готовый русский текст).
library;

import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';
import 'package:braid/src/rust/api/braid.dart' show PlatformCaps;
import 'package:braid/src/rust/privilege.dart'
    show PrivilegeStatus, SudoersPlan;

export 'package:braid/src/rust/api/braid.dart' show PlatformCaps;
export 'package:braid/src/rust/error.dart' show BridgeError, BridgeErrorKind;
export 'package:braid/src/rust/privilege.dart'
    show PrivilegeStatus, SudoersPlan;

abstract interface class BraidBridge {
  // ---------- запуск, версии, готовность ----------

  /// Версия ядра (`braid-core`).
  String coreVersion();

  /// Версия приложения: по ней ядро сверяет `client.latest` подписки.
  String clientVersion();

  /// Ядро создано. На Android до этого сервис ещё стартует.
  bool coreReady();

  /// Что умеет платформа: приложения в правилах, меню-бар, сканер QR.
  PlatformCaps platformCaps();

  /// macOS и прочий десктоп: построить ядро в [storageDir]; Android: убедиться,
  /// что его построил сервис. `true` — ядро готово.
  Future<bool> startCore(String storageDir);

  /// Поток кадров состояния: первый — сразу, дальше — на каждое изменение.
  Stream<StateFrame> watchState();

  // ---------- зеркала `Core::*` ----------

  /// `braid://…` → подписка. Ссылка другого хоста при живой подписке даёт
  /// [ReplaceRequested]: замена — только после подтверждения ([replaceLink]).
  Future<ImportOutcome> importLink(String link);
  Future<void> replaceLink(String link);
  Future<RefreshOutcome> refreshSubscription();

  Future<void> connect();
  Future<void> disconnect();

  Future<void> setEngineMode(EngineMode mode);
  Future<EngineMode> engineMode();

  Future<void> setRules(RulesChange change);

  Future<void> setSettings(Settings settings);
  Future<Settings> settings();

  /// «Проверить связь»: отчёты по всем движкам.
  Future<List<EngineProbeReport>> probeNow();

  Future<void> onNetworkChanged(String id, NetworkKind kind);
  Future<void> onInterrupted({required bool byOtherVpn});
  Future<void> setIdle({required bool idle});
  Future<void> userInput(String engine, UserInput input);

  /// Путь к файлу экспорта статистики (только админ).
  Future<String> exportStats();

  /// Восстановить сессию после перезагрузки (автозапуск). `true` — подключаемся.
  Future<bool> resume();
  Future<List<String>> recentErrors();

  /// Текущий кадр — тот же, что приходит потоком.
  Future<StateFrame> stateSnapshot();

  // ---------- сверх `Core::*` ----------

  /// Наборы и режимы подписки с исключениями — для экрана «Правила».
  Future<RulesCatalog> rulesCatalog();

  /// Ссылка `braid://…` текущей подписки (показ QR, копирование). Секрет.
  Future<String?> subscriptionLink();

  // ---------- правило sudoers (macOS) ----------

  Future<PrivilegeStatus> privilegeStatus();

  /// Что будет установлено. Ничего не ставит.
  Future<SudoersPlan> planSudoersInstall();

  /// Поставить правило — [command] должна совпасть с командой из плана.
  Future<void> applySudoersInstall(String command);
}
