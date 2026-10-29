package org.braid.vpn

/**
 * Устойчивый идентификатор сети для памяти политики ядра (`NetworkId`): FNV-1a 64 от
 * `вид|часть|часть…` — для Wi-Fi и Ethernet это шлюз и DNS, для сотовой — оператор (MCC+MNC).
 * Та же схема, что у macOS (шлюз + MAC), без разрешений на геолокацию ради SSID.
 */
object NetworkIdentity {
    private const val FNV_OFFSET = -0x340d631b7bdddcdbL // 0xcbf29ce484222325
    private const val FNV_PRIME = 0x100000001b3L

    fun id(kind: String, parts: List<String>): String {
        val key = (listOf(kind) + parts).joinToString("|")
        var hash = FNV_OFFSET
        for (byte in key.toByteArray(Charsets.UTF_8)) {
            hash = hash xor (byte.toLong() and 0xff)
            hash *= FNV_PRIME
        }
        return java.lang.Long.toHexString(hash).padStart(16, '0')
    }
}
