package org.braid.vpn

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

/**
 * Идентификатор сети для памяти политики ядра: FNV-1a 64 от `вид|часть|…`.
 * Ожидаемые значения посчитаны независимо (эталонная реализация FNV-1a на Python).
 */
class NetworkIdentityTest {
    @Test
    fun matchesReferenceFnv1a64() {
        assertEquals("a63b01f6557796a8", NetworkIdentity.id("wifi", emptyList()))
        assertEquals("b7e9af821a05a224", NetworkIdentity.id("wifi", listOf("192.168.1.1", "192.168.1.1")))
        assertEquals("4cd1373147be8acf", NetworkIdentity.id("cellular", listOf("25001")))
        assertEquals("cd65d9ed72ea10dc", NetworkIdentity.id("wired", listOf("10.0.0.1", "8.8.8.8", "1.1.1.1")))
    }

    @Test
    fun sameNetworkSameIdOtherNetworkOtherId() {
        val home = NetworkIdentity.id("wifi", listOf("192.168.1.1", "192.168.1.1"))
        assertEquals(home, NetworkIdentity.id("wifi", listOf("192.168.1.1", "192.168.1.1")))
        assertNotEquals(home, NetworkIdentity.id("wifi", listOf("192.168.0.1", "192.168.0.1")))
        assertNotEquals(home, NetworkIdentity.id("wired", listOf("192.168.1.1", "192.168.1.1")))
        assertEquals(16, home.length)
    }
}
