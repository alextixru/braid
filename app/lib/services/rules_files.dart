import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';

/// Файлы со своими правилами (десктоп: экспорт и импорт, ист. 52).
abstract interface class RulesFiles {
  /// Файловые диалоги есть: десктоп.
  bool get available;

  /// Спросить, куда сохранить, и записать; возвращает путь или `null`, если
  /// пользователь отказался.
  Future<String?> save(String suggestedName, String content);

  /// Спросить, какой файл открыть, и прочитать; `null` — отказ.
  Future<String?> open();
}

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
    // Приложение без песочницы: проверку прав macOS для диалогов пропускаем.
    await FilePicker.skipEntitlementsChecks();
    final path = await FilePicker.saveFile(
      dialogTitle: 'Сохранить свои правила',
      fileName: suggestedName,
      type: FileType.custom,
      allowedExtensions: const ['json'],
    );
    if (path == null) return null;
    await File(path).writeAsString(content);
    return path;
  }

  @override
  Future<String?> open() async {
    await FilePicker.skipEntitlementsChecks();
    final result = await FilePicker.pickFiles(
      dialogTitle: 'Открыть файл правил',
      type: FileType.custom,
      allowedExtensions: const ['json'],
    );
    final path = result?.files.single.path;
    if (path == null) return null;
    return utf8.decode(await File(path).readAsBytes());
  }
}
