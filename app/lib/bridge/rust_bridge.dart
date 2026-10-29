/// Настоящий мост: вызовы сгенерированного `flutter_rust_bridge`, структурное
/// ходит JSON-строкой в serde-форме ядра и разбирается здесь.
library;

import 'dart:convert';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/json.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';
import 'package:braid/src/rust/api/braid.dart' as api;
import 'package:braid/src/rust/api/commands.dart' as commands;
import 'package:braid/src/rust/api/privilege.dart' as privilege;

class RustBridge implements BraidBridge {
  const RustBridge();

  @override
  String coreVersion() => api.coreVersion();

  @override
  String clientVersion() => api.clientVersion();

  @override
  bool coreReady() => api.coreReady();

  @override
  PlatformCaps platformCaps() => api.platformCaps();

  @override
  Future<bool> startCore(String storageDir) =>
      api.startCore(storageDir: storageDir);

  @override
  Stream<StateFrame> watchState() => api.watchState().map(StateFrame.parse);

  @override
  Future<ImportOutcome> importLink(String link) async =>
      ImportOutcome.fromJson(_object(await commands.importLink(link: link)));

  @override
  Future<void> replaceLink(String link) => commands.replaceLink(link: link);

  @override
  Future<RefreshOutcome> refreshSubscription() async =>
      RefreshOutcome.fromJson(_object(await commands.refreshSubscription()));

  @override
  Future<void> connect() => commands.connect();

  @override
  Future<void> disconnect() => commands.disconnect();

  @override
  Future<void> setEngineMode(EngineMode mode) =>
      commands.setEngineMode(modeJson: jsonEncode(mode.toJson()));

  @override
  Future<EngineMode> engineMode() async =>
      EngineMode.fromJson(_object(await commands.engineMode()));

  @override
  Future<void> setRules(RulesChange change) =>
      commands.setRules(changeJson: jsonEncode(change.toJson()));

  @override
  Future<void> setSettings(Settings settings) =>
      commands.setSettings(settingsJson: jsonEncode(settings.toJson()));

  @override
  Future<Settings> settings() async =>
      Settings.fromJson(_object(await commands.settings()));

  @override
  Future<List<EngineProbeReport>> probeNow() async {
    final reports = asList(_decode(await commands.probeNow()), 'отчёты проб');
    return [
      for (final report in reports)
        EngineProbeReport.fromJson(asObject(report, 'отчёт пробы')),
    ];
  }

  @override
  Future<void> onNetworkChanged(String id, NetworkKind kind) =>
      commands.onNetworkChanged(id: id, kind: kind.name);

  @override
  Future<void> onInterrupted({required bool byOtherVpn}) =>
      commands.onInterrupted(byOtherVpn: byOtherVpn);

  @override
  Future<void> setIdle({required bool idle}) => commands.setIdle(idle: idle);

  @override
  Future<void> userInput(String engine, UserInput input) =>
      commands.userInput(engine: engine, inputJson: jsonEncode(input.toJson()));

  @override
  Future<String> exportStats() => commands.exportStats();

  @override
  Future<bool> resume() => commands.resume();

  @override
  Future<List<String>> recentErrors() => commands.recentErrors();

  @override
  Future<StateFrame> stateSnapshot() async =>
      StateFrame.parse(await commands.stateSnapshot());

  @override
  Future<RulesCatalog> rulesCatalog() async =>
      RulesCatalog.fromJson(_object(await commands.rulesCatalog()));

  @override
  Future<String?> subscriptionLink() => commands.subscriptionLink();

  @override
  Future<PrivilegeStatus> privilegeStatus() => privilege.privilegeStatus();

  @override
  Future<SudoersPlan> planSudoersInstall() => privilege.planSudoersInstall();

  @override
  Future<void> applySudoersInstall(String command) =>
      privilege.applySudoersInstall(command: command);

  static Object? _decode(String text) {
    try {
      return jsonDecode(text);
    } on FormatException catch (e) {
      throw FormatException('ответ ядра не JSON: ${e.message}');
    }
  }

  static Map<String, dynamic> _object(String text) =>
      asObject(_decode(text), 'ответ ядра');
}
