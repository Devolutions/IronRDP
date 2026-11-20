import { describe, it, expect } from 'vitest';
import { formatTime } from '../src/ui/format-time.js';

describe('formatTime', () => {
    // Basic formatting
    it('formats zero', () => {
        expect(formatTime(0)).toBe('0:00');
    });

    it('formats seconds only', () => {
        expect(formatTime(5_000)).toBe('0:05');
    });

    it('formats minutes and seconds', () => {
        expect(formatTime(83_000)).toBe('1:23');
    });

    it('formats exact minute boundary', () => {
        expect(formatTime(60_000)).toBe('1:00');
    });

    it('formats just under one hour', () => {
        expect(formatTime(3_599_000)).toBe('59:59');
    });

    // Hour format (H:MM:SS)
    it('formats exact hour boundary', () => {
        expect(formatTime(3_600_000)).toBe('1:00:00');
    });

    it('formats hours, minutes, and seconds', () => {
        expect(formatTime(7_261_000)).toBe('2:01:01');
    });

    it('zero-pads minutes and seconds in hour format', () => {
        expect(formatTime(3_661_000)).toBe('1:01:01');
    });

    // Edge cases
    it('returns 0:00 for negative input', () => {
        expect(formatTime(-1)).toBe('0:00');
    });

    it('returns 0:00 for NaN', () => {
        expect(formatTime(NaN)).toBe('0:00');
    });

    it('returns 0:00 for Infinity', () => {
        expect(formatTime(Infinity)).toBe('0:00');
    });

    it('returns 0:00 for -Infinity', () => {
        expect(formatTime(-Infinity)).toBe('0:00');
    });

    // Sub-second truncation
    it('truncates sub-second ms to 0:00', () => {
        expect(formatTime(999)).toBe('0:00');
    });

    it('truncates at minute boundary', () => {
        expect(formatTime(59_999)).toBe('0:59');
    });

    // Large values
    it('formats 24+ hours', () => {
        expect(formatTime(86_400_000)).toBe('24:00:00');
    });
});
