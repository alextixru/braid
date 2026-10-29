/// Разбор `THIRD_PARTY.md` для экрана «О программе» (ист. 21, R34i).
///
/// Файл в корне репозитория — единственный источник: компоненты, лицензии и
/// ограничения читаются из него, а не пересказываются в коде. В приложение
/// он попадает копией в assets (`assets/third_party.md`). Разбираем ровно то,
/// что в нём есть: заголовок, абзацы и таблицы Markdown.
library;

sealed class ThirdPartyBlock {
  const ThirdPartyBlock();
}

/// Абзац.
final class ThirdPartyText extends ThirdPartyBlock {
  const ThirdPartyText(this.text);

  final String text;
}

/// Таблица: шапка и строки; в ячейках остаётся разметка Markdown.
final class ThirdPartyTable extends ThirdPartyBlock {
  const ThirdPartyTable({required this.header, required this.rows});

  final List<String> header;
  final List<List<String>> rows;
}

class ThirdPartySection {
  const ThirdPartySection({required this.title, required this.blocks});

  final String title;
  final List<ThirdPartyBlock> blocks;
}

class ThirdPartyDoc {
  const ThirdPartyDoc({
    required this.title,
    required this.intro,
    required this.sections,
  });

  factory ThirdPartyDoc.parse(String markdown) {
    var title = '';
    final intro = <String>[];
    final sections = <ThirdPartySection>[];

    String? sectionTitle;
    var blocks = <ThirdPartyBlock>[];
    final paragraph = <String>[];
    List<List<String>>? table;

    void flushParagraph() {
      if (paragraph.isEmpty) return;
      final text = paragraph.join(' ');
      paragraph.clear();
      if (sectionTitle == null) {
        intro.add(text);
      } else {
        blocks.add(ThirdPartyText(text));
      }
    }

    void flushTable() {
      final rows = table;
      table = null;
      if (rows == null || rows.isEmpty) return;
      blocks.add(ThirdPartyTable(header: rows.first, rows: rows.sublist(1)));
    }

    void flushSection() {
      flushParagraph();
      flushTable();
      final name = sectionTitle;
      if (name != null) {
        sections.add(
          ThirdPartySection(title: name, blocks: List.unmodifiable(blocks)),
        );
      }
      blocks = <ThirdPartyBlock>[];
    }

    for (final raw in markdown.split('\n')) {
      final line = raw.trim();
      if (line.startsWith('## ')) {
        flushSection();
        sectionTitle = line.substring(3).trim();
      } else if (line.startsWith('# ') &&
          title.isEmpty &&
          sectionTitle == null) {
        flushParagraph();
        title = line.substring(2).trim();
      } else if (line.startsWith('|')) {
        flushParagraph();
        final cells = _cells(line);
        if (!_isSeparator(cells)) (table ??= []).add(cells);
      } else if (line.isEmpty) {
        flushParagraph();
        flushTable();
      } else {
        flushTable();
        paragraph.add(line);
      }
    }
    flushSection();
    return ThirdPartyDoc(
      title: title,
      intro: List.unmodifiable(intro),
      sections: List.unmodifiable(sections),
    );
  }

  final String title;
  final List<String> intro;
  final List<ThirdPartySection> sections;

  static List<String> _cells(String line) {
    var inner = line;
    if (inner.startsWith('|')) inner = inner.substring(1);
    if (inner.endsWith('|')) inner = inner.substring(0, inner.length - 1);
    const escaped = '\u0000';
    return [
      for (final cell in inner.replaceAll(r'\|', escaped).split('|'))
        cell.replaceAll(escaped, '|').trim(),
    ];
  }

  static bool _isSeparator(List<String> cells) =>
      cells.isNotEmpty &&
      cells.every((c) => RegExp(r'^:?-{3,}:?$').hasMatch(c));
}

/// Текст без разметки Markdown: жирный, код, ссылки.
String plainText(String markdown) {
  return markdown
      .replaceAllMapped(RegExp(r'\[([^\]]+)\]\([^)]*\)'), (m) => m[1]!)
      .replaceAll('**', '')
      .replaceAll('`', '');
}

/// Лицензия запрещает коммерческое использование (PolyForm Noncommercial).
bool isNoncommercial(String license) =>
    plainText(license).toLowerCase().contains('noncommercial');
