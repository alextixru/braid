/// Данные экрана «Статистика» (ист. 63) и выгрузка журнала в файл.
///
/// Единственный путь к журналу — `export_stats`: ядро склеивает дни в один
/// файл и отдаёт его путь. Каталог ядра на Android выбирает сервис, а не окно,
/// поэтому читать журнал напрямую из Dart нельзя.
library;

import 'dart:convert';
import 'dart:io';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/screens/stats/stats_report.dart';
import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';

/// Файлы выгрузки. Тесты подменяют их памятью.
abstract interface class StatsFiles {
  /// Содержимое файла экспорта.
  Future<String> read(String path);

  /// Предложить пользователю сохранить файл; вернуть путь или `null`, если он
  /// отказался.
  Future<String?> offer(String sourcePath, {required String suggestedName});
}

class SystemStatsFiles implements StatsFiles {
  const SystemStatsFiles();

  @override
  Future<String> read(String path) async =>
      utf8.decode(await File(path).readAsBytes(), allowMalformed: true);

  @override
  Future<String?> offer(
    String sourcePath, {
    required String suggestedName,
  }) async {
    final bytes = await File(sourcePath).readAsBytes();
    // Приложение без песочницы: проверку прав macOS для диалогов пропускаем
    // (так же, как для файла своих правил).
    await FilePicker.skipEntitlementsChecks();
    // С байтами плагин сам записывает файл: на десктопе — по выбранному пути,
    // на Android — через системный выбор места.
    return FilePicker.saveFile(
      dialogTitle: 'Сохранить журнал',
      fileName: suggestedName,
      type: FileType.custom,
      allowedExtensions: const ['jsonl'],
      bytes: Uint8List.fromList(bytes),
    );
  }
}

class StatsService {
  StatsService({required this.controller, required this.files});

  final BraidController controller;
  final StatsFiles files;

  /// Имя файла, которое предлагается при сохранении.
  static const exportFileName = 'braid-journal.jsonl';

  /// Свежая сводка за 7 дней.
  Future<StatsReport> load() async {
    final path = await controller.exportStats();
    final text = await files.read(path);
    return aggregateJournal(text, now: controller.now());
  }

  /// «Экспорт»: `export_stats` и предложение сохранить файл. Возвращает путь
  /// или `null`, если пользователь отказался.
  Future<String?> export() async {
    final path = await controller.exportStats();
    return files.offer(path, suggestedName: exportFileName);
  }
}
