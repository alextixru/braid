package org.braid.vpn

/**
 * Домен из текста share-меню (ист. 54): первый токен, похожий на URL или домен.
 * `https://site.com/…` → `site.com`; `www.` отбрасывается — своё правило и так действует
 * на поддомены. Голые IP-адреса и обычные слова — не домен (`null`).
 */
object DomainExtractor {
    private val scheme = Regex("^[A-Za-z][A-Za-z0-9+.-]*://")
    private val label = Regex("^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$")
    private val tld = Regex("^[a-z]{2,63}$")
    /** Знаки вокруг ссылки в тексте: скобки, кавычки, точка в конце предложения. */
    private const val TRIM = "<>\"'()[]{},;!?«»."

    fun extract(text: String?): String? {
        if (text.isNullOrBlank()) return null
        return text.split(Regex("\\s+")).firstNotNullOfOrNull { hostOf(it.trim { c -> c in TRIM }) }
    }

    /** Хост из URL или голого домена; `null`, если на домен не похоже. */
    fun hostOf(token: String): String? {
        if (token.isEmpty()) return null
        var s = scheme.replace(token, "")
        s = s.substringBefore('/').substringBefore('?').substringBefore('#')
        if ('@' in s) s = s.substringAfterLast('@')
        s = s.substringBefore(':').trimEnd('.').lowercase()
        s = s.removePrefix("www.")
        return s.takeIf(::isDomain)
    }

    private fun isDomain(s: String): Boolean {
        val labels = s.split('.')
        if (labels.size < 2 || !labels.all(label::matches)) return false
        // Зона из букв: отсекает IP-адреса и «1.5».
        return tld.matches(labels.last())
    }
}
