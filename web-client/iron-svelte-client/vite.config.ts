import { sveltekit } from '@sveltejs/kit/vite';
import type { UserConfig } from 'vite';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

const config: UserConfig = {
	mode: process.env.MODE || 'development',
	plugins: [sveltekit(), wasm(), topLevelAwait()]
};

export default config;
