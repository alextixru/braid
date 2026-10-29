// Что хранится на устройстве для CSQTT (ист. 14, 78): токен VK и свои хеши
// звонков. Подписка их не несёт, ядро не хранит (память движка живёт до
// выхода из приложения), поэтому читать и писать их умеет только экран —
// в защищённое хранилище устройства.

import 'package:braid/services/vk/vk_credentials.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Map<String, String> raw;
  late SecureVkCredentialStore store;

  setUp(() {
    raw = {};
    FlutterSecureStorage.setMockInitialValues(raw);
    store = const SecureVkCredentialStore();
  });

  test('токена нет, пока не вошли', () async {
    expect(await store.readToken(), isNull);
  });

  test('токен записывается, читается и удаляется', () async {
    await store.writeToken('  vk1.a.synthetic-token  ');
    expect(await store.readToken(), 'vk1.a.synthetic-token');
    expect(raw.values, contains('vk1.a.synthetic-token'));

    await store.deleteToken();
    expect(await store.readToken(), isNull);
    expect(raw, isEmpty);
  });

  test('пустой токен — это «не вошли»', () async {
    await store.writeToken('   ');
    expect(await store.readToken(), isNull);
  });

  test('хеши записываются и читаются в том же порядке', () async {
    expect(await store.readHashes(), isEmpty);
    await store.writeHashes(['hash-one-aaaaaaaa', 'hash-two-bbbbbbbb']);
    expect(await store.readHashes(), [
      'hash-one-aaaaaaaa',
      'hash-two-bbbbbbbb',
    ]);
  });

  test('пустой список хешей стирает запись', () async {
    await store.writeHashes(['hash-one-aaaaaaaa']);
    await store.writeHashes(const []);
    expect(await store.readHashes(), isEmpty);
    expect(raw, isEmpty);
  });

  test('испорченная запись хешей читается как пустой список', () async {
    for (final broken in ['не json', '{"a":1}', '[1, 2]', '']) {
      raw
        ..clear()
        ..[SecureVkCredentialStore.hashesKey] = broken;
      expect(await store.readHashes(), isEmpty, reason: broken);
    }
  });
}
