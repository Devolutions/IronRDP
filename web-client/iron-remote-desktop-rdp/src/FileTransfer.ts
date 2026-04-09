/**
 * File metadata for file transfer operations.
 *
 * When remote copies files, this metadata is provided to the client.
 * File names are limited to 259 characters per MS-RDPECLIP spec.
 */
export interface FileInfo {
    /** File basename (including extension, without directory path) */
    name: string;
    /**
     * Relative directory path within the copied collection.
     * Uses `\` as separator (matching the Windows wire protocol convention).
     * Absent or undefined for root-level files.
     *
     * Per MS-RDPECLIP 3.1.1.2, file lists use relative paths to describe
     * directory structure (e.g., `"temp\\subdir"`).
     *
     * @example
     * // File at root level:
     * { name: "readme.txt", size: 100, lastModified: 0 }
     * // File inside "docs" folder:
     * { name: "report.pdf", path: "docs", size: 2048, lastModified: 0 }
     * // File inside nested folder:
     * { name: "image.png", path: "docs\\images", size: 4096, lastModified: 0 }
     */
    path?: string;
    /** File size in bytes (0 for empty files or unknown size) */
    size: number;
    /**
     * Last write time as a JavaScript timestamp (milliseconds since Unix epoch,
     * same as `File.lastModified`). The WASM layer converts to Windows FILETIME
     * for the RDP protocol. 0 indicates unknown or not applicable.
     */
    lastModified: number;
    /**
     * Whether this entry represents a directory rather than a file.
     * Directory entries are used to describe the folder structure of a copied
     * collection and typically have size 0.
     */
    isDirectory?: boolean;
}

/**
 * File contents request from remote (when remote requests file upload from client).
 *
 * The client should read the requested file chunk and respond via CLIPRDR.
 */
export interface FileContentsRequest {
    /** Stream identifier for this file transfer */
    streamId: number;
    /** File index in the file list (0-based) */
    index: number;
    /**
     * FileContentsFlags bitmask - use FileContentsFlags.SIZE or FileContentsFlags.RANGE
     * - FileContentsFlags.SIZE (0x1): Request file size
     * - FileContentsFlags.RANGE (0x2): Request byte range
     */
    flags: number;
    /** Byte offset for RANGE requests */
    position: number;
    /** Number of bytes requested for RANGE requests */
    size: number;
    /** Optional clipboard lock ID from LockClipData PDU */
    dataId?: number;
}

/**
 * File contents response from remote (when remote sends file download to client).
 *
 * This is the response to a client's file contents request.
 */
export interface FileContentsResponse {
    /** Stream identifier for this file transfer */
    streamId: number;
    /** If true, the request failed (data unavailable/access denied) */
    isError: boolean;
    /**
     * Response data:
     * - For SIZE requests: 8-byte little-endian u64
     * - For RANGE requests: requested byte range
     */
    data: Uint8Array;
}
