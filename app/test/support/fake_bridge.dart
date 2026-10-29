/// Подделка моста для тестов: ведёт журнал вызовов и ведёт себя как ядро
/// ровно настолько, чтобы экраны видели последствия команд (набор
/// выключился, домен добавился). Настоящего ядра здесь нет — его форму
/// проверяют эталонные документы.
library;

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';

import 'states.dart';

const desktopCaps = PlatformCaps(perApp: false, menuBar: true, qrScan: false);
const androidCaps = PlatformCaps(perApp: true, menuBar: false, qrScan: true);

const noPrivilege = PrivilegeStatus(
  supported: false,
  ruleInstalled: false,
  helperCurrent: false,
);

const sudoersPlanFixture = SudoersPlan(
  path: '/etc/sudoers.d/braid',
  installedHelper: '/Library/PrivilegedHelperTools/org.braid.helper',
  rule: 'alex ALL=(root) NOPASSWD: sha256:0123abcd /Library/PrivilegedHelperTools/org.braid.helper *',
  command: "/usr/bin/install -m 0440 -o root -g wheel /tmp/braid-sudoers /etc/sudoers.d/braid",
);

class FakeBridge implements BraidBridge {
  FakeBridge({
    CoreState? state,
    this.ready = true,
    this.caps = desktopCaps,
    this.storedSettings = const Settings(),
    this.catalog = catalogFixture,
    this.privilege = noPrivilege,
    this.link = 'braid://current-subscription',
    this.sudoersPlan = sudoersPlanFixture,
  }) : state = state ?? idleState();

  // ---------- сценарий ----------

  CoreState state;
  bool ready;
  PlatformCaps caps;
  Settings storedSettings;
  RulesCatalog catalog;
  PrivilegeStatus privilege;
  SudoersPlan sudoersPlan;

  /// Ссылка текущей подписки; `null` — подписки нет.
  String? link;

  String client = '0.1.0';
  String core = '0.1.0';

  /// Следующие вызовы бросят эту ошибку (и сбросят её).
  BridgeError? connectError;
  BridgeError? importError;
  BridgeError? refreshError;
  BridgeError? settingsError;
  BridgeError? rulesError;
  BridgeError? sudoersError;
  BridgeError? startFailure;
  BridgeError? privilegeError;

  /// Что вернёт `refreshSubscription`.
  RefreshOutcome refreshOutcome = const NotModified();

  /// Держит `connect()` «в полёте», пока тест не завершит completer.
  Completer<void>? connectGate;

  /// Держит `refreshSubscription()` «в полёте».
  Completer<void>? refreshGate;

  /// Если задан — вызывается вместо стандартного разбора ссылки.
  Future<ImportOutcome> Function(String link)? importHandler;

  /// Хост новой подписки при замене.
  String replacementHostName = 'Хельсинки';

  List<EngineProbeReport> probeReports = const [];
  bool resumeResult = false;

  // ---------- журнал ----------

  final calls = <String>[];
  final importedLinks = <String>[];
  final replacedLinks = <String>[];
  final rulesChanges = <RulesChange>[];
  final engineModes = <EngineMode>[];
  final settingsWrites = <Settings>[];
  final userInputs = <(String, UserInput)>[];
  final appliedSudoers = <String>[];
  String? startedWith;

  final _bus = StreamController<StateFrame>.broadcast();

  void _call(String name) => calls.add(name);

  /// Опубликовать кадр, как это делает ядро.
  void emit({CoreState? next, bool? isReady}) {
    if (next != null) state = next;
    if (isReady != null) ready = isReady;
    _bus.add(StateFrame(ready: ready, state: state));
  }

  /// Сломанный кадр: поток отдаёт ошибку разбора.
  void emitBroken(String message) => _bus.addError(FormatException(message));

  CoreState _withRules(RulesView Function(RulesView) change) {
    return CoreState(
      session: state.session,
      active: state.active,
      engines: state.engines,
      rules: change(state.rules),
      subscription: state.subscription,
      network: state.network,
      banners: state.banners,
      admin: state.admin,
    );
  }

  // ---------- запуск ----------

  @override
  String coreVersion() => core;

  @override
  String clientVersion() => client;

  @override
  bool coreReady() => ready;

  @override
  PlatformCaps platformCaps() => caps;

  @override
  Future<bool> startCore(String storageDir) async {
    _call('startCore');
    startedWith = storageDir;
    final failure = startFailure;
    if (failure != null) throw failure;
    return ready;
  }

  @override
  Stream<StateFrame> watchState() {
    _call('watchState');
    return Stream.multi((controller) {
      controller.add(StateFrame(ready: ready, state: state));
      final sub = _bus.stream.listen(
        controller.add,
        onError: controller.addError,
      );
      controller.onCancel = sub.cancel;
    });
  }

  // ---------- подписка ----------

  @override
  Future<ImportOutcome> importLink(String text) async {
    _call('importLink');
    importedLinks.add(text);
    final error = importError;
    if (error != null) {
      importError = null;
      throw error;
    }
    final handler = importHandler;
    if (handler != null) return handler(text);
    if (!text.startsWith('braid://')) {
      throw const BridgeError(
        kind: BridgeErrorKind.invalidLink,
        message: 'Это не ссылка Braid',
        details: [],
      );
    }
    if (state.hasSubscription && text != link) {
      return ReplaceRequested(
        currentHost: state.subscription.hostName ?? state.subscription.hostId!,
        newHost: 'new.example',
      );
    }
    link = text;
    emit(next: idleState());
    return const Imported();
  }

  @override
  Future<void> replaceLink(String text) async {
    _call('replaceLink');
    replacedLinks.add(text);
    link = text;
    emit(
      next: idleState(
        subscription: SubscriptionView(
          hostId: 'helsinki',
          hostName: replacementHostName,
          version: 1,
        ),
        rules: state.rules,
      ),
    );
  }

  @override
  Future<RefreshOutcome> refreshSubscription() async {
    _call('refreshSubscription');
    final gate = refreshGate;
    if (gate != null) await gate.future;
    final error = refreshError;
    if (error != null) {
      refreshError = null;
      throw error;
    }
    return refreshOutcome;
  }

  // ---------- сессия ----------

  @override
  Future<void> connect() async {
    _call('connect');
    final error = connectError;
    if (error != null) {
      connectError = null;
      throw error;
    }
    emit(next: _session(const SessionConnecting()));
    final gate = connectGate;
    if (gate != null) await gate.future;
    emit(next: _session(const SessionConnected(), active: 'awg'));
  }

  @override
  Future<void> disconnect() async {
    _call('disconnect');
    emit(next: _session(const SessionIdle()));
  }

  CoreState _session(SessionState session, {String? active}) {
    return CoreState(
      session: session,
      active: active,
      engines: state.engines,
      rules: state.rules,
      subscription: state.subscription,
      network: state.network,
      banners: state.banners,
      admin: state.admin,
    );
  }

  @override
  Future<void> setEngineMode(EngineMode mode) async {
    _call('setEngineMode');
    engineModes.add(mode);
    final pinned = mode is EngineModePinned ? mode.engineId : null;
    emit(
      next: CoreState(
        session: state.session,
        active: state.active,
        engines: [
          for (final e in state.engines)
            EngineView(
              id: e.id,
              kind: e.kind,
              state: e.state,
              health: e.health,
              pinned: e.id == pinned,
              warm: e.warm,
              userAction: e.userAction,
            ),
        ],
        rules: state.rules,
        subscription: state.subscription,
        network: state.network,
        banners: state.banners,
        admin: state.admin,
      ),
    );
  }

  @override
  Future<EngineMode> engineMode() async {
    _call('engineMode');
    final pinned = state.engines.where((e) => e.pinned);
    return pinned.isEmpty
        ? const EngineMode.auto()
        : EngineMode.pinned(pinned.first.id);
  }

  // ---------- правила ----------

  @override
  Future<void> setRules(RulesChange change) async {
    _call('setRules');
    rulesChanges.add(change);
    final error = rulesError;
    if (error != null) {
      rulesError = null;
      throw error;
    }
    emit(
      next: _withRules((rules) {
        switch (change) {
          case ToggleSet(:final id, :final on):
            return RulesView(
              sets: [
                for (final s in rules.sets)
                  RuleSetView(
                    id: s.id,
                    name: s.name,
                    enabled: s.id == id ? on : s.enabled,
                    domains: s.domains,
                    ips: s.ips,
                  ),
              ],
              mode: null,
              userDomains: rules.userDomains,
              userCidrs: rules.userCidrs,
              apps: rules.apps,
            );
          case SelectMode(:final id):
            final mode = catalog.modeById(id)!;
            return RulesView(
              sets: [
                for (final s in rules.sets)
                  RuleSetView(
                    id: s.id,
                    name: s.name,
                    enabled: mode.sets.contains(s.id),
                    domains: s.domains,
                    ips: s.ips,
                  ),
              ],
              mode: id,
              userDomains: rules.userDomains,
              userCidrs: rules.userCidrs,
              apps: rules.apps,
            );
          case AddUserDomain(:final domain):
            return _rulesWith(rules, domains: [...rules.userDomains, domain]);
          case RemoveUserDomain(:final domain):
            return _rulesWith(
              rules,
              domains: [
                for (final d in rules.userDomains)
                  if (d != domain) d,
              ],
            );
          case AddUserCidr(:final cidr):
            return _rulesWith(rules, cidrs: [...rules.userCidrs, cidr]);
          case RemoveUserCidr(:final cidr):
            return _rulesWith(
              rules,
              cidrs: [
                for (final c in rules.userCidrs)
                  if (c != cidr) c,
              ],
            );
          case SetApps(:final apps):
            return _rulesWith(rules, apps: apps);
        }
      }),
    );
  }

  RulesView _rulesWith(
    RulesView rules, {
    List<String>? domains,
    List<String>? cidrs,
    AppRules? apps,
  }) {
    return RulesView(
      sets: rules.sets,
      mode: rules.mode,
      userDomains: domains ?? rules.userDomains,
      userCidrs: cidrs ?? rules.userCidrs,
      apps: apps ?? rules.apps,
    );
  }

  @override
  Future<RulesCatalog> rulesCatalog() async {
    _call('rulesCatalog');
    return catalog;
  }

  @override
  Future<String?> subscriptionLink() async {
    _call('subscriptionLink');
    return link;
  }

  // ---------- настройки ----------

  @override
  Future<Settings> settings() async {
    _call('settings');
    return storedSettings;
  }

  @override
  Future<void> setSettings(Settings settings) async {
    _call('setSettings');
    settingsWrites.add(settings);
    final error = settingsError;
    if (error != null) {
      settingsError = null;
      throw error;
    }
    storedSettings = settings;
  }

  // ---------- прочее ----------

  @override
  Future<List<EngineProbeReport>> probeNow() async {
    _call('probeNow');
    return probeReports;
  }

  @override
  Future<void> onNetworkChanged(String id, NetworkKind kind) async =>
      _call('onNetworkChanged');

  @override
  Future<void> onInterrupted({required bool byOtherVpn}) async =>
      _call('onInterrupted');

  @override
  Future<void> setIdle({required bool idle}) async => _call('setIdle');

  @override
  Future<void> userInput(String engine, UserInput input) async {
    _call('userInput');
    userInputs.add((engine, input));
  }

  @override
  Future<String> exportStats() async {
    _call('exportStats');
    return '/tmp/braid-journal-export.jsonl';
  }

  @override
  Future<bool> resume() async {
    _call('resume');
    return resumeResult;
  }

  @override
  Future<List<String>> recentErrors() async {
    _call('recentErrors');
    return const [];
  }

  @override
  Future<StateFrame> stateSnapshot() async {
    _call('stateSnapshot');
    return StateFrame(ready: ready, state: state);
  }

  // ---------- sudoers ----------

  @override
  Future<PrivilegeStatus> privilegeStatus() async {
    _call('privilegeStatus');
    final error = privilegeError;
    if (error != null) throw error;
    return privilege;
  }

  @override
  Future<SudoersPlan> planSudoersInstall() async {
    _call('planSudoersInstall');
    return sudoersPlan;
  }

  @override
  Future<void> applySudoersInstall(String command) async {
    _call('applySudoersInstall');
    appliedSudoers.add(command);
    final error = sudoersError;
    if (error != null) {
      sudoersError = null;
      throw error;
    }
    privilege = const PrivilegeStatus(
      supported: true,
      ruleInstalled: true,
      helperCurrent: true,
    );
  }

  Future<void> close() => _bus.close();
}
