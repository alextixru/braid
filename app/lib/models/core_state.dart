/// Состояние ядра для экрана — зеркало `braid_core::state` (JSON в serde-форме
/// ядра). Эталоны документов лежат в `test/fixtures/` и проверяются и здесь,
/// и в Rust (`app/rust/src/wire.rs`, `contract.rs`).
library;

import 'dart:convert';

import 'package:braid/models/json.dart';
import 'package:braid/util/value.dart';

/// Кадр ленты состояний: `ready` — ядро создано (на Android его строит
/// сервис, и оно появляется позже окна).
class StateFrame with Value {
  const StateFrame({required this.ready, required this.state});

  final bool ready;
  final CoreState state;

  factory StateFrame.fromJson(Map<String, dynamic> json) {
    final state = json['state'];
    return StateFrame(
      ready: reqBool(json, 'ready'),
      // Аварийный кадр моста — только `{"ready":false}`.
      state: state == null
          ? const CoreState()
          : CoreState.fromJson(asObject(state, 'state')),
    );
  }

  /// Разбор кадра из строки, которую отдаёт мост.
  factory StateFrame.parse(String text) {
    final Object? decoded;
    try {
      decoded = jsonDecode(text);
    } on FormatException catch (e) {
      throw FormatException('кадр состояния не JSON: ${e.message}');
    }
    return StateFrame.fromJson(asObject(decoded, 'кадр состояния'));
  }

  @override
  List<Object?> get props => [ready, state];
}

class CoreState with Value {
  const CoreState({
    this.session = const SessionIdle(),
    this.active,
    this.engines = const [],
    this.rules = const RulesView(),
    this.subscription = const SubscriptionView(),
    this.network,
    this.banners = const [],
    this.admin = false,
  });

  final SessionState session;

  /// Идентификатор движка, несущего трафик.
  final String? active;
  final List<EngineView> engines;
  final RulesView rules;
  final SubscriptionView subscription;

  /// Непрозрачный идентификатор сети (хеш) — для памяти и автоподключения.
  final String? network;
  final List<CoreBanner> banners;
  final bool admin;

  factory CoreState.fromJson(Map<String, dynamic> json) {
    return CoreState(
      session: SessionState.fromJson(asObject(json['session'], 'session')),
      active: optString(json, 'active'),
      engines: List.unmodifiable(
        asList(
          json['engines'],
          'engines',
        ).map((e) => EngineView.fromJson(asObject(e, 'engines[]'))),
      ),
      rules: RulesView.fromJson(asObject(json['rules'], 'rules')),
      subscription: SubscriptionView.fromJson(
        asObject(json['subscription'], 'subscription'),
      ),
      network: optString(json, 'network'),
      banners: List.unmodifiable(
        asList(
          json['banners'],
          'banners',
        ).map((b) => CoreBanner.tryParse(asObject(b, 'banners[]'))).nonNulls,
      ),
      admin: reqBool(json, 'admin'),
    );
  }

  /// Подписка загружена (есть хост) — иначе единственный экран «ссылка или QR».
  bool get hasSubscription => subscription.hostId != null;

  EngineView? engineById(String? id) {
    if (id == null) return null;
    for (final engine in engines) {
      if (engine.id == id) return engine;
    }
    return null;
  }

  @override
  List<Object?> get props => [
    session,
    active,
    engines,
    rules,
    subscription,
    network,
    banners,
    admin,
  ];
}

// ---------- сессия ----------

sealed class SessionState with Value {
  const SessionState();

  factory SessionState.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'state')) {
      'idle' => const SessionIdle(),
      'needs_link' => const SessionNeedsLink(),
      'connecting' => const SessionConnecting(),
      'connected' => const SessionConnected(),
      'no_host' => SessionNoHost(reqString(json, 'last_error')),
      'interrupted' => SessionInterrupted(
        byOtherVpn: reqBool(json, 'by_other_vpn'),
      ),
      final other => throw FormatException(
        'состояние сессии «$other» неизвестно',
      ),
    };
  }
}

/// Не подключён.
final class SessionIdle extends SessionState {
  const SessionIdle();
  @override
  List<Object?> get props => const [];
}

/// Подписки нет — подключаться нечем.
final class SessionNeedsLink extends SessionState {
  const SessionNeedsLink();
  @override
  List<Object?> get props => const [];
}

final class SessionConnecting extends SessionState {
  const SessionConnecting();
  @override
  List<Object?> get props => const [];
}

final class SessionConnected extends SessionState {
  const SessionConnected();
  @override
  List<Object?> get props => const [];
}

/// Все движки мертвы; защищённый трафик напрямую не уходит.
final class SessionNoHost extends SessionState {
  const SessionNoHost(this.lastError);
  final String lastError;
  @override
  List<Object?> get props => [lastError];
}

/// Интерфейс забрал другой VPN или разрешение отозвано.
final class SessionInterrupted extends SessionState {
  const SessionInterrupted({required this.byOtherVpn});
  final bool byOtherVpn;
  @override
  List<Object?> get props => [byOtherVpn];
}

// ---------- движки ----------

enum EngineStatus {
  stopped,
  starting,
  connected,
  degraded,
  dead,

  /// Капча, нужны хеши или вход VK.
  waitingUser;

  static EngineStatus parse(String text) {
    return switch (text) {
      'stopped' => stopped,
      'starting' => starting,
      'connected' => connected,
      'degraded' => degraded,
      'dead' => dead,
      'waiting_user' => waitingUser,
      _ => throw FormatException('состояние движка «$text» неизвестно'),
    };
  }
}

class Health with Value {
  const Health({this.rttMs, this.lossPct = 0, this.handshakeAgeS});

  final int? rttMs;
  final int lossPct;
  final int? handshakeAgeS;

  factory Health.fromJson(Map<String, dynamic> json) {
    return Health(
      rttMs: optInt(json, 'rtt_ms'),
      lossPct: reqInt(json, 'loss_pct'),
      handshakeAgeS: optInt(json, 'handshake_age_s'),
    );
  }

  @override
  List<Object?> get props => [rttMs, lossPct, handshakeAgeS];
}

/// Что движок ждёт от пользователя.
sealed class UserAction with Value {
  const UserAction();

  factory UserAction.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'action')) {
      'captcha' => CaptchaAction(
        url: reqString(json, 'url'),
        session: reqString(json, 'session'),
      ),
      'need_hashes' => const NeedHashesAction(),
      'vk_login' => const VkLoginAction(),
      final other => throw FormatException('действие «$other» неизвестно'),
    };
  }
}

final class CaptchaAction extends UserAction {
  const CaptchaAction({required this.url, required this.session});
  final String url;
  final String session;
  @override
  List<Object?> get props => [url, session];
}

final class NeedHashesAction extends UserAction {
  const NeedHashesAction();
  @override
  List<Object?> get props => const [];
}

final class VkLoginAction extends UserAction {
  const VkLoginAction();
  @override
  List<Object?> get props => const [];
}

class EngineView with Value {
  const EngineView({
    required this.id,
    required this.kind,
    this.state = EngineStatus.stopped,
    this.health = const Health(),
    this.pinned = false,
    this.warm = false,
    this.userAction,
  });

  final String id;

  /// `awg`, `csqtt` или `kind` слота под третий движок.
  final String kind;
  final EngineStatus state;
  final Health health;
  final bool pinned;
  final bool warm;
  final UserAction? userAction;

  factory EngineView.fromJson(Map<String, dynamic> json) {
    final action = json['user_action'];
    return EngineView(
      id: reqString(json, 'id'),
      kind: reqString(json, 'kind'),
      state: EngineStatus.parse(reqString(json, 'state')),
      health: Health.fromJson(asObject(json['health'], 'health')),
      pinned: reqBool(json, 'pinned'),
      warm: reqBool(json, 'warm'),
      userAction: action == null
          ? null
          : UserAction.fromJson(asObject(action, 'user_action')),
    );
  }

  @override
  List<Object?> get props => [
    id,
    kind,
    state,
    health,
    pinned,
    warm,
    userAction,
  ];
}

// ---------- правила ----------

/// Правила по приложениям (только Android).
sealed class AppRules with Value {
  const AppRules();

  factory AppRules.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'mode')) {
      'all' => const AppRulesAll(),
      'allowed' => AppRulesAllowed(reqStrings(json, 'packages')),
      'excluded' => AppRulesExcluded(reqStrings(json, 'packages')),
      final other => throw FormatException(
        'режим приложений «$other» неизвестен',
      ),
    };
  }

  Map<String, dynamic> toJson();

  /// Пакеты выбранного режима; для «все» — пусто.
  List<String> get packages;
}

/// Все приложения — по спискам.
final class AppRulesAll extends AppRules {
  const AppRulesAll();
  @override
  Map<String, dynamic> toJson() => {'mode': 'all'};
  @override
  List<String> get packages => const [];
  @override
  List<Object?> get props => const [];
}

/// Только эти приложения целиком в туннель.
final class AppRulesAllowed extends AppRules {
  const AppRulesAllowed(this.packages);
  @override
  final List<String> packages;
  @override
  Map<String, dynamic> toJson() => {'mode': 'allowed', 'packages': packages};
  @override
  List<Object?> get props => [packages];
}

/// Эти приложения всегда напрямую.
final class AppRulesExcluded extends AppRules {
  const AppRulesExcluded(this.packages);
  @override
  final List<String> packages;
  @override
  Map<String, dynamic> toJson() => {'mode': 'excluded', 'packages': packages};
  @override
  List<Object?> get props => [packages];
}

class RuleSetView with Value {
  const RuleSetView({
    required this.id,
    required this.name,
    required this.enabled,
    required this.domains,
    required this.ips,
  });

  final String id;
  final String name;
  final bool enabled;
  final int domains;
  final int ips;

  factory RuleSetView.fromJson(Map<String, dynamic> json) {
    return RuleSetView(
      id: reqString(json, 'id'),
      name: reqString(json, 'name'),
      enabled: reqBool(json, 'enabled'),
      domains: reqInt(json, 'domains'),
      ips: reqInt(json, 'ips'),
    );
  }

  @override
  List<Object?> get props => [id, name, enabled, domains, ips];
}

class RulesView with Value {
  const RulesView({
    this.sets = const [],
    this.mode,
    this.userDomains = const [],
    this.userCidrs = const [],
    this.apps = const AppRulesAll(),
  });

  final List<RuleSetView> sets;

  /// `id` выбранного режима-пресета; `null` — своя комбинация («Своё»).
  final String? mode;
  final List<String> userDomains;
  final List<String> userCidrs;
  final AppRules apps;

  factory RulesView.fromJson(Map<String, dynamic> json) {
    return RulesView(
      sets: List.unmodifiable(
        asList(
          json['sets'],
          'sets',
        ).map((s) => RuleSetView.fromJson(asObject(s, 'sets[]'))),
      ),
      mode: optString(json, 'mode'),
      userDomains: reqStrings(json, 'user_domains'),
      userCidrs: reqStrings(json, 'user_cidrs'),
      apps: AppRules.fromJson(asObject(json['apps'], 'apps')),
    );
  }

  @override
  List<Object?> get props => [sets, mode, userDomains, userCidrs, apps];
}

// ---------- подписка и баннеры ----------

class SubscriptionView with Value {
  const SubscriptionView({
    this.hostId,
    this.hostName,
    this.version,
    this.updatedAt,
    this.lastRefreshError,
    this.revoked = false,
  });

  final String? hostId;
  final String? hostName;
  final int? version;
  final DateTime? updatedAt;
  final String? lastRefreshError;
  final bool revoked;

  factory SubscriptionView.fromJson(Map<String, dynamic> json) {
    return SubscriptionView(
      hostId: optString(json, 'host_id'),
      hostName: optString(json, 'host_name'),
      version: optInt(json, 'version'),
      updatedAt: optTime(json, 'updated_at'),
      lastRefreshError: optString(json, 'last_refresh_error'),
      revoked: reqBool(json, 'revoked'),
    );
  }

  @override
  List<Object?> get props => [
    hostId,
    hostName,
    version,
    updatedAt,
    lastRefreshError,
    revoked,
  ];
}

/// Плашка сверху главного экрана. Имя `CoreBanner`, чтобы не путать с
/// виджетом `Banner` из Flutter.
sealed class CoreBanner with Value {
  const CoreBanner();

  /// `null` — вид баннера этой версии приложению неизвестен.
  static CoreBanner? tryParse(Map<String, dynamic> json) {
    return switch (reqString(json, 'banner')) {
      'update_available' => UpdateAvailableBanner(
        latest: reqString(json, 'latest'),
        url: optString(json, 'url'),
      ),
      'needs_hashes' => const NeedsHashesBanner(),
      'needs_vk_login' => const NeedsVkLoginBanner(),
      'subscription_stale' => SubscriptionStaleBanner(
        since: reqTime(json, 'since'),
      ),
      'revoked' => const RevokedBanner(),
      'other_vpn' => OtherVpnBanner(iface: reqString(json, 'iface')),
      'autoconnect_ask' => AutoconnectAskBanner(
        network: reqString(json, 'network'),
      ),
      _ => null,
    };
  }
}

final class UpdateAvailableBanner extends CoreBanner {
  const UpdateAvailableBanner({required this.latest, this.url});
  final String latest;
  final String? url;
  @override
  List<Object?> get props => [latest, url];
}

final class NeedsHashesBanner extends CoreBanner {
  const NeedsHashesBanner();
  @override
  List<Object?> get props => const [];
}

final class NeedsVkLoginBanner extends CoreBanner {
  const NeedsVkLoginBanner();
  @override
  List<Object?> get props => const [];
}

final class SubscriptionStaleBanner extends CoreBanner {
  const SubscriptionStaleBanner({required this.since});
  final DateTime since;
  @override
  List<Object?> get props => [since];
}

final class RevokedBanner extends CoreBanner {
  const RevokedBanner();
  @override
  List<Object?> get props => const [];
}

final class OtherVpnBanner extends CoreBanner {
  const OtherVpnBanner({required this.iface});
  final String iface;
  @override
  List<Object?> get props => [iface];
}

/// Новая сеть с правилом «спрашивать»: подключаться здесь?
final class AutoconnectAskBanner extends CoreBanner {
  const AutoconnectAskBanner({required this.network});
  final String network;
  @override
  List<Object?> get props => [network];
}
