import { describe, expect, it } from 'vitest';
import { Config } from './Config';

describe('Config WebSocket protocols', () => {
    it('accepts the existing constructor options without WebSocket protocols', () => {
        const config = new Config(
            { username: 'user', password: 'pass' },
            { address: 'wss://test', authToken: 'token' },
            { destination: 'test:3389', serverDomain: '', extensions: [] },
        );

        expect(config.webSocketProtocols).toEqual([]);
    });

    it('retains explicitly configured WebSocket protocols', () => {
        const config = new Config(
            { username: 'user', password: 'pass' },
            { address: 'wss://test', authToken: 'token' },
            { destination: 'test:3389', serverDomain: '', extensions: [], webSocketProtocols: ['binary', 'v2'] },
        );

        expect(config.webSocketProtocols).toEqual(['binary', 'v2']);
    });
});
