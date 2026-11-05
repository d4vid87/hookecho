package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files

class TransferFilesTest {
    @Test fun oversizedImportsStopAtTheLimitAndRemoveTheirPartialCopy() {
        val root = Files.createTempDirectory("hookecho-transfer-limit").toFile()
        try {
            val limit = TransferFiles.importLimit("palette")
            var consumed = 0L
            val endless = object : InputStream() {
                override fun read(): Int { consumed++; return 1 }
                override fun read(bytes: ByteArray, offset: Int, length: Int): Int {
                    consumed += length
                    bytes.fill(1, offset, offset + length)
                    return length
                }
            }
            try {
                TransferFiles.copyImport(root, root, "palette", "large.pal", endless)
                fail("Oversized imports must fail")
            } catch (error: IllegalArgumentException) {
                assertTrue(error.message!!.contains("limit"))
            }
            assertEquals(limit + 1, consumed)
            assertTrue(File(root, "imports").listFiles()!!.isEmpty())
            val exact = ByteArray(limit.toInt())
            val file = TransferFiles.copyImport(root, root, "palette", "exact.pal", exact.inputStream())
            assertEquals(limit, file.length())
        } finally { root.deleteRecursively() }
    }

    @Test fun referencedAssetsSurviveCacheRemovalAndNeverOverwriteTheSameName() {
        val root = Files.createTempDirectory("hookecho-transfer").toFile()
        try {
            val files = File(root, "files")
            val cache = File(root, "cache")
            val palette = TransferFiles.copyImport(files, cache, "palette", "custom.pal", "palette".byteInputStream())
            val other = TransferFiles.copyImport(files, cache, "palette", "custom.pal", "other".byteInputStream())
            val sound = TransferFiles.copyImport(files, cache, "sound", "alert.wav", "sound".byteInputStream())
            val settings = TransferFiles.copyImport(files, cache, "settings", "bundle.json", "settings".byteInputStream())
            assertNotEquals(palette, other)
            cache.deleteRecursively()
            assertEquals("palette", palette.readText())
            assertEquals("other", other.readText())
            assertEquals("sound", sound.readText())
            assertFalse(settings.exists())
            assertEquals("pal", palette.extension)
            assertEquals("wav", sound.extension)
        } finally { root.deleteRecursively() }
    }

    @Test fun providerNamesCannotEscapeStorageOrCorruptTheHandoff() {
        val root = Files.createTempDirectory("hookecho-transfer").toFile()
        try {
            val files = File(root, "files")
            for (name in listOf("../../evil.pal", "/absolute.pal", "..\\escape.pal", "bad\tname\n.pal", "界".repeat(300) + ".pal", "..", "")) {
                val file = TransferFiles.copyImport(files, root, "palette", name, "ok".byteInputStream())
                assertEquals(File(files, "imports").canonicalFile, file.canonicalFile.parentFile)
                assertFalse(file.name.any { it.isISOControl() })
            }
            val before = File(files, "imports").listFiles()!!.toSet()
            val broken = object : InputStream() { override fun read(): Int = throw IOException("read failed") }
            try {
                TransferFiles.copyImport(files, root, "palette", "partial.pal", broken)
                fail("A failed copy must fail the import")
            } catch (_: IOException) { }
            assertEquals(before, File(files, "imports").listFiles()!!.toSet())
        } finally { root.deleteRecursively() }
    }

    @Test fun exportHandoffOnlyAcceptsCompletedFilesInsideTheSharedDirectory() {
        val root = Files.createTempDirectory("hookecho-transfer").toFile()
        try {
            val exports = File(root, "data/exports").apply { mkdirs() }
            for ((name, mime) in listOf("view.png" to "image/png", "loop.gif" to "image/gif",
                "settings.json" to "application/json", "diagnostics.json" to "application/json",
                "field.csv" to "text/csv", "field.tif" to "image/tiff", "field.nc" to "application/x-netcdf")) {
                val file = File(exports, name).apply { writeText("complete") }
                assertEquals(file.canonicalFile, TransferFiles.exportFile(root, file.path))
                assertEquals(mime, TransferFiles.mime(file))
            }
            val privateFile = File(root, "settings.json").apply { writeText("private") }
            for (file in listOf(privateFile, File(exports, "missing.png"), exports)) {
                try {
                    TransferFiles.exportFile(root, file.path)
                    fail("Private, missing and directory paths must not be shared")
                } catch (_: IllegalArgumentException) { }
            }
        } finally { root.deleteRecursively() }
    }
}
