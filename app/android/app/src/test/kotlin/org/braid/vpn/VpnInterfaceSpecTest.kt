package org.braid.vpn

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Описание `VpnService.Builder` по JSON `requestEstablish` из Rust (ист. 3, 42, 43;
 * оговорка тикета 05): адрес, DNS, маршруты, IPv6 напрямую, режимы «по спискам» с
 * исключёнными приложениями и «по приложениям» без самого Braid.
 */
class VpnInterfaceSpecTest {
    private val self = "org.braid.vpn"

    /** JSON в том виде, в каком его пишет `EstablishRequest::to_json` (config.rs). */
    private fun json(allowed: String = "[]", disallowed: String = "[]") =
        """{"address":"10.255.0.2","prefix":32,"mtu":1380,"dns":["10.255.0.1"],""" +
            """"routes":["10.255.0.1/32","198.18.0.0/15","203.0.113.0/24"],""" +
            """"allowed":$allowed,"disallowed":$disallowed}"""

    @Test
    fun listsModeBuildsAddressDnsRoutesAndIpv6Bypass() {
        val spec = VpnInterfaceSpec.fromJson(json(), self)
        assertEquals(
            listOf(
                "addAddress 10.255.0.2/32",
                "setMtu 1380",
                "addDnsServer 10.255.0.1",
                "addRoute 10.255.0.1/32",
                "addRoute 198.18.0.0/15",
                "addRoute 203.0.113.0/24",
                "allowFamily AF_INET6",
            ),
            spec.describe(),
        )
    }

    @Test
    fun excludedAppStaysDirectEvenWithRuleRoutes() {
        // Банк исключён: интерфейс его не видит, хотя маршруты правил (203.0.113.0/24) стоят.
        val spec = VpnInterfaceSpec.fromJson(json(disallowed = """["com.bank"]"""), self)
        val ops = spec.describe()
        assertTrue(ops.contains("addDisallowedApplication com.bank"))
        assertTrue(ops.contains("addRoute 203.0.113.0/24"))
        assertFalse(ops.any { it.startsWith("addAllowedApplication") })
    }

    @Test
    fun perAppModeNeverAllowsBraidItself() {
        // Сокеты CSQTT не проходят protect: попади Braid в allowed — они ушли бы в свой же туннель.
        val spec = VpnInterfaceSpec.fromJson(json(allowed = """["com.x","$self","com.y"]"""), self)
        val allowed = spec.describe().filter { it.startsWith("addAllowedApplication") }
        assertEquals(listOf("addAllowedApplication com.x", "addAllowedApplication com.y"), allowed)
        assertFalse(spec.describe().any { it.startsWith("addDisallowedApplication") })
    }

    @Test(expected = IllegalArgumentException::class)
    fun allowedAndDisallowedTogetherAreRejected() {
        VpnInterfaceSpec.fromJson(json(allowed = """["com.x"]""", disallowed = """["com.bank"]"""), self)
    }

    @Test(expected = IllegalArgumentException::class)
    fun mtuOutOfRangeIsRejected() {
        VpnInterfaceSpec.fromJson(
            """{"address":"10.255.0.2","prefix":32,"mtu":100,"dns":[],"routes":[],"allowed":[],"disallowed":[]}""",
            self,
        )
    }

    @Test
    fun cidrParsesPrefixAndDefaultsBareAddressToHost() {
        assertEquals("198.18.0.0" to 15, VpnInterfaceSpec.cidr("198.18.0.0/15"))
        assertEquals("10.255.0.1" to 32, VpnInterfaceSpec.cidr("10.255.0.1"))
    }

    @Test(expected = IllegalArgumentException::class)
    fun cidrWithBadPrefixIsRejected() {
        VpnInterfaceSpec.cidr("10.0.0.0/33")
    }
}
