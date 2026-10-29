package org.braid.vpn

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Домен из текста share-меню (ист. 54): `https://site.com/…` → `site.com`. */
class DomainExtractorTest {
    @Test
    fun urlFromShareMenuGivesItsHost() {
        assertEquals("site.com", DomainExtractor.extract("https://site.com/some/path?x=1#top"))
    }

    @Test
    fun hostIsLowercasedWithoutWwwPortAndUserinfo() {
        assertEquals("example.com", DomainExtractor.extract("Смотри: https://www.Example.COM:8443/x"))
        assertEquals("host.io", DomainExtractor.extract("https://user:pass@host.io/"))
    }

    @Test
    fun bareDomainAndPunctuationAroundIt() {
        assertEquals("mail.google.com", DomainExtractor.extract("mail.google.com"))
        assertEquals("site.com", DomainExtractor.extract("(https://site.com)."))
        assertEquals("site.com", DomainExtractor.extract("ссылка: site.com, глянь"))
    }

    @Test
    fun firstDomainLikeTokenWins() {
        assertEquals("one.org", DomainExtractor.extract("см one.org и two.org"))
    }

    @Test
    fun ipAddressesPlainWordsAndEmptyTextAreNotDomains() {
        assertNull(DomainExtractor.extract("192.168.1.1"))
        assertNull(DomainExtractor.extract("просто текст без ссылок"))
        assertNull(DomainExtractor.extract("1.5"))
        assertNull(DomainExtractor.extract("   "))
        assertNull(DomainExtractor.extract(null))
    }
}
