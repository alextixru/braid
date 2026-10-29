/// Настройки клиента — зеркало `braid_core::Settings` (ист. 55, 58, 59, 33, 77).
library;

import 'package:braid/models/json.dart';
import 'package:braid/util/value.dart';

/// Что делать при входе в сеть.
enum Autoconnect {
  connect,
  ask,
  never;

  static Autoconnect parse(String text) {
    return switch (text) {
      'connect' => connect,
      'ask' => ask,
      'never' => never,
      _ => throw FormatException('правило сети «$text» неизвестно'),
    };
  }

  String toJson() => name;
}

/// Режим DNS (ист. 77).
enum DnsMode {
  /// Через туннель — только домены правил; остальное напрямую (по умолчанию).
  rulesOnly,

  /// Весь DNS через туннель, как в Amnezia.
  allViaTunnel;

  static DnsMode parse(String text) {
    return switch (text) {
      'rules_only' => rulesOnly,
      'all_via_tunnel' => allViaTunnel,
      _ => throw FormatException('режим DNS «$text» неизвестен'),
    };
  }

  String toJson() => switch (this) {
    rulesOnly => 'rules_only',
    allViaTunnel => 'all_via_tunnel',
  };
}

class Settings with Value {
  const Settings({
    this.autostart = false,
    this.autoconnectDefault = Autoconnect.ask,
    this.autoconnectNetworks = const {},
    this.notifySwitches = false,
    this.killSwitch = true,
    this.dnsMode = DnsMode.rulesOnly,
  });

  final bool autostart;

  /// Для сетей, которых нет в [autoconnectNetworks].
  final Autoconnect autoconnectDefault;

  /// Известные сети: идентификатор → правило.
  final Map<String, Autoconnect> autoconnectNetworks;

  /// Уведомления о переключении движка; по умолчанию выключены.
  final bool notifySwitches;

  /// Kill switch по умолчанию включён (G03, ист. 6 и 33) — как в ядре и в
  /// эталоне `test/fixtures/settings.json`; выключение — явный выбор.
  final bool killSwitch;
  final DnsMode dnsMode;

  factory Settings.fromJson(Map<String, dynamic> json) {
    final networks = asObject(
      json['autoconnect_networks'],
      'autoconnect_networks',
    );
    return Settings(
      autostart: reqBool(json, 'autostart'),
      autoconnectDefault: Autoconnect.parse(
        reqString(json, 'autoconnect_default'),
      ),
      autoconnectNetworks: Map.unmodifiable({
        for (final entry in networks.entries)
          entry.key: Autoconnect.parse(entry.value as String),
      }),
      notifySwitches: reqBool(json, 'notify_switches'),
      killSwitch: reqBool(json, 'kill_switch'),
      dnsMode: DnsMode.parse(reqString(json, 'dns_mode')),
    );
  }

  Map<String, dynamic> toJson() => {
    'autostart': autostart,
    'autoconnect_default': autoconnectDefault.toJson(),
    'autoconnect_networks': {
      for (final entry in autoconnectNetworks.entries)
        entry.key: entry.value.toJson(),
    },
    'notify_switches': notifySwitches,
    'kill_switch': killSwitch,
    'dns_mode': dnsMode.toJson(),
  };

  Settings copyWith({
    bool? autostart,
    Autoconnect? autoconnectDefault,
    Map<String, Autoconnect>? autoconnectNetworks,
    bool? notifySwitches,
    bool? killSwitch,
    DnsMode? dnsMode,
  }) {
    return Settings(
      autostart: autostart ?? this.autostart,
      autoconnectDefault: autoconnectDefault ?? this.autoconnectDefault,
      autoconnectNetworks: autoconnectNetworks ?? this.autoconnectNetworks,
      notifySwitches: notifySwitches ?? this.notifySwitches,
      killSwitch: killSwitch ?? this.killSwitch,
      dnsMode: dnsMode ?? this.dnsMode,
    );
  }

  @override
  List<Object?> get props => [
    autostart,
    autoconnectDefault,
    autoconnectNetworks,
    notifySwitches,
    killSwitch,
    dnsMode,
  ];
}
