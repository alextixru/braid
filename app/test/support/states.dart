/// Готовые состояния ядра для тестов: подписка Франкфурта с наборами из
/// эталонной подписки `braid-proto` (google, cursor; режимы work и home).
library;

import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';

const awgEngine = EngineView(id: 'awg', kind: 'awg');
const csqttEngine = EngineView(id: 'csqtt', kind: 'csqtt');

const rulesWork = RulesView(
  sets: [
    RuleSetView(
      id: 'google',
      name: 'Google',
      enabled: true,
      domains: 3,
      ips: 2,
    ),
    RuleSetView(
      id: 'cursor',
      name: 'Cursor',
      enabled: true,
      domains: 2,
      ips: 0,
    ),
  ],
  mode: 'work',
);

const frankfurt = SubscriptionView(
  hostId: 'frankfurt',
  hostName: 'Франкфурт',
  version: 7,
);

/// Подписка загружена, не подключено.
CoreState idleState({
  SessionState session = const SessionIdle(),
  List<EngineView>? engines,
  RulesView rules = rulesWork,
  SubscriptionView subscription = frankfurt,
  String? network,
  List<CoreBanner> banners = const [],
  bool admin = false,
  String? active,
}) {
  return CoreState(
    session: session,
    active: active,
    engines: engines ?? const [awgEngine, csqttEngine],
    rules: rules,
    subscription: subscription,
    network: network,
    banners: banners,
    admin: admin,
  );
}

/// Подключено через AWG.
CoreState connectedState({
  List<CoreBanner> banners = const [],
  RulesView rules = rulesWork,
  String? network,
  bool admin = false,
}) {
  return idleState(
    session: const SessionConnected(),
    active: 'awg',
    rules: rules,
    network: network,
    banners: banners,
    admin: admin,
    engines: const [
      EngineView(
        id: 'awg',
        kind: 'awg',
        state: EngineStatus.connected,
        health: Health(rttMs: 38, handshakeAgeS: 12),
      ),
      EngineView(
        id: 'csqtt',
        kind: 'csqtt',
        warm: true,
        state: EngineStatus.connected,
      ),
    ],
  );
}

/// Подписки нет — единственный экран «Вставь ссылку или отсканируй QR».
const noSubscriptionState = CoreState(session: SessionNeedsLink());

const catalogFixture = RulesCatalog(
  sets: [
    SetEntry(
      id: 'google',
      name: 'Google',
      domains: 3,
      ips: 2,
      excludeDomains: ['mail.google.com'],
      excludeIps: ['142.251.1.0/24'],
    ),
    SetEntry(id: 'cursor', name: 'Cursor', domains: 2, ips: 0),
  ],
  modes: [
    ModeEntry(
      id: 'work',
      name: 'Работа',
      sets: ['google', 'cursor'],
      excludeDomains: ['calendar.google.com'],
      isDefault: true,
    ),
    ModeEntry(id: 'home', name: 'Дом', sets: ['google']),
  ],
);
