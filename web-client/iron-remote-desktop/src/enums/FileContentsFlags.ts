/**
 * FileContentsFlags values per [MS-RDPECLIP] 2.2.5.3
 *
 * Used in FileContentsRequest to specify the type of operation.
 */
export const FileContentsFlags = {
    /**
     * Request file size (8-byte unsigned integer).
     * When set: position must be 0, size must be 8.
     */
    SIZE: 0x1,

    /**
     * Request byte range from file.
     * When set: position and size define the requested range.
     */
    RANGE: 0x2,
} as const;

export type FileContentsFlags = (typeof FileContentsFlags)[keyof typeof FileContentsFlags];
