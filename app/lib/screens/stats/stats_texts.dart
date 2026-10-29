/// Тексты экрана «Статистика».
library;

import 'package:braid/models/core_state.dart';
import 'package:braid/util/format.dart';

/// Причина переключения словами. Код, которого эта версия не знает, остаётся
/// как есть: ядро могло научиться новому.
String reasonText(String code) => switch (code) {
  'dead' => 'нет ответа',
  'degraded' => 'деградация',
  'stall' => 'данные не идут',
  'manual' => 'вручную',
  'priority' => 'возврат на приоритетный',
  'memory' => 'память сети',
  'startup' => 'старт',
  _ => code,
};

/// Название движка из журнала (там его `id`): как в интерфейсе, если движок
/// есть в подписке, иначе — прописными.
String journalEngineName(String id, List<EngineView> engines) {
  for (final engine in engines) {
    if (engine.id == id) return engineLabel(engine, engines);
  }
  return id.toUpperCase();
}

/// «потери 70 %, отклик 2300 мс» — то, что известно о движке в момент решения.
String? switchMetricsText({int? rttMs, required int lossPct}) {
  final parts = [
    if (lossPct > 0) 'потери $lossPct %',
    if (rttMs != null) 'отклик $rttMs мс',
  ];
  return parts.isEmpty ? null : parts.join(', ');
}
