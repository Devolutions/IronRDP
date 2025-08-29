export enum ClipboardApiSupported {
    // Full clipboard API support (read and write text and images)
    Full,
    // Text-only support (Firefox v125-v126)
    TextOnly,
    // Text-only support, but only writing data received from the server (Firefox < v125)
    TextOnlyServerOnly,
    // Clipboard API is not supported at all
    None,
}
