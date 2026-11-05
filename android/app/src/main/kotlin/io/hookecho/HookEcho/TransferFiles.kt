package io.hookecho.HookEcho

import java.io.File
import java.io.InputStream
import java.util.Locale

/** File ownership for the SAF bridge; provider display names are never trusted as paths. */
internal object TransferFiles {
    fun importLimit(kind: String): Long = when (kind) {
        "palette" -> 2L * 1024 * 1024
        "marker", "settings" -> 8L * 1024 * 1024
        "gpx", "case" -> 16L * 1024 * 1024
        "sound" -> 32L * 1024 * 1024
        "gis" -> 64L * 1024 * 1024
        else -> error("Unsupported import type")
    }

    fun copyImport(filesDir: File, cacheDir: File, kind: String, name: String, input: InputStream): File {
        val limit = importLimit(kind)
        val root = if (kind == "palette" || kind == "sound") filesDir else cacheDir
        val dir = File(root, "imports").apply { check(isDirectory || mkdirs()) { "Cannot create import storage" } }
        val safeName = name.substringAfterLast('/').substringAfterLast('\\')
            .map { if (it.isISOControl() || it == ':') '_' else it }.joinToString("")
            .takeLast(64).takeUnless { it.isBlank() || it == "." || it == ".." } ?: "import"
        val dest = File.createTempFile("import-", "-$safeName", dir)
        try {
            dest.outputStream().use { output ->
                val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                var copied = 0L
                while (true) {
                    val count = input.read(buffer, 0, minOf(buffer.size.toLong(), limit - copied + 1).toInt())
                    if (count < 0) break
                    copied += count
                    require(copied <= limit) { "Import exceeds ${limit / (1024 * 1024)} MB limit" }
                    output.write(buffer, 0, count)
                }
            }
            return dest
        } catch (error: Exception) {
            dest.delete() // Only the new, app-owned partial copy belongs to this operation.
            throw error
        }
    }

    fun exportFile(filesDir: File, path: String): File {
        val file = File(path).canonicalFile
        require(file.parentFile == File(filesDir, "data/exports").canonicalFile && file.isFile) {
            "Export file is unavailable"
        }
        return file
    }

    fun mime(file: File): String = when (file.extension.lowercase(Locale.ROOT)) {
        "png" -> "image/png"
        "gif" -> "image/gif"
        "mp4" -> "video/mp4"
        "webm" -> "video/webm"
        "json", "geojson" -> "application/json"
        "csv" -> "text/csv"
        "gpx" -> "application/gpx+xml"
        "pal", "pal3", "txt" -> "text/plain"
        "tif", "tiff" -> "image/tiff"
        "nc" -> "application/x-netcdf"
        "html" -> "text/html"
        "pdf" -> "application/pdf"
        else -> "application/octet-stream"
    }
}
