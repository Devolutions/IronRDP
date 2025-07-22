export interface ConfigParser {
    get_str(key: string): string | null;
    get_int(key: string): number | null;
}
