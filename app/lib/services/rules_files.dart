import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:braid/services/platform_info.dart';
import 'package:file_picker/file_picker.dart';

/// Файлы со своими правилами (экспорт и импорт, ист. 52).
abstract interface class RulesFiles {
  /// Файловые диалоги есть: десктоп и Android.
  bool get available;

  /// Спросить, куда сохранить, и записать; возвращает путь или `null`, если
  /// пользователь отказался.
  Future<String?> save(String suggestedName, String content);

  /// Спросить, какой файл открыть, и прочитать; `null` — отказ.
  Future<String?> open();
}

/// Какие файловые диалоги есть на этой платформе. На Android экспорт и импорт
/// нужны не меньше, чем на десктопе: хранилище ядра лежит в `noBackupFilesDir`
/// и в системную резервную копию не попадает (ист. 52).
RulesFiles rulesFilesFor(PlatformInfo platform) =>
    platform.isDesktop || platform.isAndroid
    ? const PickerRulesFiles()
    : const NoRulesFiles();

class NoRulesFiles implements RulesFiles {
  const NoRulesFiles();

  @override
  bool get available => false;

  @override
  Future<String?> save(String suggestedName, String content) async => null;

  @override
  Future<String?> open() async => null;
}

class PickerRulesFiles implements RulesFiles {
  const PickerRulesFiles();

  @override
  bool get available => true;

  @override
  Future<String?> save(String suggestedName, String content) async {
    // Приложение без песочницы: проверку прав macOS для диалогов пропускаем
    // (на других платформах вызов ничего не делает).
    await FilePicker.skipEntitlementsChecks();
    // С байтами плагин сам записывает файл: на десктопе — по выбранному пути,
    // на Android — через системный выбор места (так же, как журнал).
    return FilePicker.saveFile(
      dialogTitle: 'Сохранить свои правила',
      fileName: suggestedName,
      type: FileType.custom,
      allowedExtensions: const ['json'],
      bytes: Uint8List.fromList(utf8.encode(content)),
    );
  }

  @override
  Future<String?> open() async {
    await FilePicker.skipEntitlementsChecks();
    // На Android выбранный файл приходит байтами (путь может быть копией в
    // кэше или отсутствовать), на десктопе — путём.
    final result = await FilePicker.pickFiles(
      dialogTitle: 'Открыть файл правил',
      type: FileType.custom,
      allowedExtensions: const ['json'],
      withData: true,
    );
    final file = result?.files.single;
    if (file == null) return null;
    final bytes = file.bytes ?? await File(file.path!).readAsBytes();
    return utf8.decode(bytes);
  }
}
