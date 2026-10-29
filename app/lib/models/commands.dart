/// Команды ядру в serde-форме ядра: что приложение шлёт мосту строкой JSON.
/// Эталоны — `test/fixtures/commands.json`, их же читает Rust-сторона.
library;

import 'package:braid/models/core_state.dart';
import 'package:braid/models/json.dart';
import 'package:braid/util/value.dart';

/// Изменение правил (`Core::set_rules`).
sealed class RulesChange with Value {
  const RulesChange();

  const factory RulesChange.toggleSet(String id, {required bool on}) =
      ToggleSet;
  const factory RulesChange.selectMode(String id) = SelectMode;
  const factory RulesChange.addUserDomain(String domain) = AddUserDomain;
  const factory RulesChange.removeUserDomain(String domain) = RemoveUserDomain;
  const factory RulesChange.addUserCidr(String cidr) = AddUserCidr;
  const factory RulesChange.removeUserCidr(String cidr) = RemoveUserCidr;
  const factory RulesChange.setApps(AppRules apps) = SetApps;

  Map<String, dynamic> toJson();
}

final class ToggleSet extends RulesChange {
  const ToggleSet(this.id, {required this.on});
  final String id;
  final bool on;
  @override
  Map<String, dynamic> toJson() => {'change': 'toggle_set', 'id': id, 'on': on};
  @override
  List<Object?> get props => [id, on];
}

final class SelectMode extends RulesChange {
  const SelectMode(this.id);
  final String id;
  @override
  Map<String, dynamic> toJson() => {'change': 'select_mode', 'id': id};
  @override
  List<Object?> get props => [id];
}

final class AddUserDomain extends RulesChange {
  const AddUserDomain(this.domain);
  final String domain;
  @override
  Map<String, dynamic> toJson() => {
    'change': 'add_user_domain',
    'domain': domain,
  };
  @override
  List<Object?> get props => [domain];
}

final class RemoveUserDomain extends RulesChange {
  const RemoveUserDomain(this.domain);
  final String domain;
  @override
  Map<String, dynamic> toJson() => {
    'change': 'remove_user_domain',
    'domain': domain,
  };
  @override
  List<Object?> get props => [domain];
}

final class AddUserCidr extends RulesChange {
  const AddUserCidr(this.cidr);
  final String cidr;
  @override
  Map<String, dynamic> toJson() => {'change': 'add_user_cidr', 'cidr': cidr};
  @override
  List<Object?> get props => [cidr];
}

final class RemoveUserCidr extends RulesChange {
  const RemoveUserCidr(this.cidr);
  final String cidr;
  @override
  Map<String, dynamic> toJson() => {'change': 'remove_user_cidr', 'cidr': cidr};
  @override
  List<Object?> get props => [cidr];
}

final class SetApps extends RulesChange {
  const SetApps(this.apps);
  final AppRules apps;
  @override
  Map<String, dynamic> toJson() => {
    'change': 'set_apps',
    'apps': apps.toJson(),
  };
  @override
  List<Object?> get props => [apps];
}

/// Выбор движка: сам по приоритету или закреплён вручную.
sealed class EngineMode with Value {
  const EngineMode();

  const factory EngineMode.auto() = EngineModeAuto;
  const factory EngineMode.pinned(String engineId) = EngineModePinned;

  factory EngineMode.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'mode')) {
      'auto' => const EngineModeAuto(),
      'pinned' => EngineModePinned(reqString(json, 'engine')),
      final other => throw FormatException('режим движка «$other» неизвестен'),
    };
  }

  Map<String, dynamic> toJson();
}

final class EngineModeAuto extends EngineMode {
  const EngineModeAuto();
  @override
  Map<String, dynamic> toJson() => {'mode': 'auto'};
  @override
  List<Object?> get props => const [];
}

final class EngineModePinned extends EngineMode {
  const EngineModePinned(this.engineId);
  final String engineId;
  @override
  Map<String, dynamic> toJson() => {'mode': 'pinned', 'engine': engineId};
  @override
  List<Object?> get props => [engineId];
}

/// Ответ пользователя движку (`Core::user_input`).
sealed class UserInput with Value {
  const UserInput();

  const factory UserInput.captchaResult(String value) = CaptchaResult;
  const factory UserInput.vkHashes(List<String> values) = VkHashes;
  const factory UserInput.vkToken(String value) = VkToken;

  Map<String, dynamic> toJson();
}

final class CaptchaResult extends UserInput {
  const CaptchaResult(this.value);
  final String value;
  @override
  Map<String, dynamic> toJson() => {'input': 'captcha_result', 'value': value};
  @override
  List<Object?> get props => [value];
}

final class VkHashes extends UserInput {
  const VkHashes(this.values);
  final List<String> values;
  @override
  Map<String, dynamic> toJson() => {'input': 'vk_hashes', 'values': values};
  @override
  List<Object?> get props => [values];
}

/// Пустая строка — выход из VK.
final class VkToken extends UserInput {
  const VkToken(this.value);
  final String value;
  @override
  Map<String, dynamic> toJson() => {'input': 'vk_token', 'value': value};
  @override
  List<Object?> get props => [value];
}

/// Тип сети для `Core::on_network_changed`.
enum NetworkKind { wifi, cellular, wired, other }
