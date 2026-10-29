import 'dart:convert';
import 'dart:io';

/// Эталонный документ из `test/fixtures/` — его же читает Rust-сторона моста
/// (`app/rust/src/wire.rs`, `contract.rs`). Тесты запускаются из корня `app/`.
String fixtureText(String name) =>
    File('test/fixtures/$name').readAsStringSync();

Object? fixtureJson(String name) => jsonDecode(fixtureText(name));

Map<String, dynamic> fixtureObject(String name) =>
    fixtureJson(name)! as Map<String, dynamic>;
