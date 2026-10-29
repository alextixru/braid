import 'package:shared_preferences/shared_preferences.dart';

/// Мелочи интерфейса, которые помнит сам экран, а не ядро: что пользователь
/// уже отклонил. Настройки VPN хранит ядро.
abstract interface class UiPrefs {
  /// Пользователь отказался от правила sudoers — не предлагать при подключении.
  Future<bool> sudoersOfferDeclined();
  Future<void> setSudoersOfferDeclined(bool value);

  /// Версия, о которой баннер «Доступно обновление» уже закрыт.
  Future<String?> dismissedUpdate();
  Future<void> setDismissedUpdate(String version);
}

class SharedUiPrefs implements UiPrefs {
  const SharedUiPrefs();

  static const _sudoers = 'sudoers_offer_declined';
  static const _update = 'dismissed_update';

  @override
  Future<bool> sudoersOfferDeclined() async =>
      (await SharedPreferences.getInstance()).getBool(_sudoers) ?? false;

  @override
  Future<void> setSudoersOfferDeclined(bool value) async {
    await (await SharedPreferences.getInstance()).setBool(_sudoers, value);
  }

  @override
  Future<String?> dismissedUpdate() async =>
      (await SharedPreferences.getInstance()).getString(_update);

  @override
  Future<void> setDismissedUpdate(String version) async {
    await (await SharedPreferences.getInstance()).setString(_update, version);
  }
}

/// В памяти — для тестов и запуска без диска.
class MemoryUiPrefs implements UiPrefs {
  bool sudoersDeclined = false;
  String? updateDismissed;

  @override
  Future<bool> sudoersOfferDeclined() async => sudoersDeclined;

  @override
  Future<void> setSudoersOfferDeclined(bool value) async =>
      sudoersDeclined = value;

  @override
  Future<String?> dismissedUpdate() async => updateDismissed;

  @override
  Future<void> setDismissedUpdate(String version) async =>
      updateDismissed = version;
}
