/// Что экран хранит для CSQTT: токен VK и свои хеши звонков (ист. 14, 78).
///
/// Подписка их не несёт, а ядро держит только в памяти движка (до выхода из
/// приложения), поэтому после перезапуска их возвращает экран — из защищённого
/// хранилища устройства (Keychain на macOS, Keystore на Android). Ни токен, ни
/// хеши не пишутся в логи и сообщения об ошибках (правило 4).
library;

import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

abstract interface class VkCredentialStore {
  /// Токен VK; `null` — не входили.
  Future<String?> readToken();

  Future<void> writeToken(String token);

  Future<void> deleteToken();

  /// Хеши звонков в порядке ввода.
  Future<List<String>> readHashes();

  /// Пустой список стирает запись.
  Future<void> writeHashes(List<String> hashes);
}

/// Защищённое хранилище устройства через `flutter_secure_storage`.
class SecureVkCredentialStore implements VkCredentialStore {
  const SecureVkCredentialStore([this._storage = _defaultStorage]);

  static const tokenKey = 'braid.vk.token';
  static const hashesKey = 'braid.vk.hashes';

  /// macOS: связка ключей входа без Data Protection Keychain — тот требует
  /// подписи с командой разработчика, а сборка Braid подписана ad-hoc (так же
  /// сделано в FOCSQ).
  static const _defaultStorage = FlutterSecureStorage(
    mOptions: MacOsOptions(usesDataProtectionKeychain: false),
  );

  final FlutterSecureStorage _storage;

  @override
  Future<String?> readToken() async {
    final token = (await _storage.read(key: tokenKey))?.trim();
    return token == null || token.isEmpty ? null : token;
  }

  @override
  Future<void> writeToken(String token) async {
    final value = token.trim();
    if (value.isEmpty) return deleteToken();
    await _storage.write(key: tokenKey, value: value);
  }

  @override
  Future<void> deleteToken() => _storage.delete(key: tokenKey);

  @override
  Future<List<String>> readHashes() async {
    final text = await _storage.read(key: hashesKey);
    if (text == null || text.isEmpty) return const [];
    try {
      final decoded = jsonDecode(text);
      if (decoded is List && decoded.every((item) => item is String)) {
        return List<String>.unmodifiable(decoded.cast<String>());
      }
    } on FormatException {
      // Запись испорчена: лучше начать с пустого списка, чем падать на экране.
    }
    return const [];
  }

  @override
  Future<void> writeHashes(List<String> hashes) async {
    if (hashes.isEmpty) return _storage.delete(key: hashesKey);
    await _storage.write(key: hashesKey, value: jsonEncode(hashes));
  }
}
