import adapter from '@sveltejs/adapter-static';
import preprocess from 'svelte-preprocess';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	// Consult https://github.com/sveltejs/svelte-preprocess
	// for more information about preprocessors
	preprocess: preprocess(),

	kit: {
		adapter: adapter({
			// default options are shown. On some platforms
			// these options are set automatically — see below
			pages: process.env.MODE === "tauri" ? "build/tauri" : 'build/browser',
			assets: process.env.MODE === "tauri" ? "build/tauri" : 'build/browser',
			fallback: null,
			precompress: false,
			strict: true
		  })
		// appDir: process.env.MODE === "tauri" ? "tauri" : "_app",
	}
};

export default config;
