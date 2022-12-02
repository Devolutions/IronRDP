import { g as getContext, c as create_ssr_component, b as subscribe, e as escape } from '../../chunks/index.js';

/**
 * @type {import('$app/stores').getStores}
 */
const getStores = () => {
	const stores = getContext('__svelte__');

	const readonly_stores = {
		page: {
			subscribe: stores.page.subscribe
		},
		navigating: {
			subscribe: stores.navigating.subscribe
		},
		updated: stores.updated
	};

	// TODO remove this for 1.0
	Object.defineProperties(readonly_stores, {
		preloading: {
			get() {
				console.error('stores.preloading is deprecated; use stores.navigating instead');
				return {
					subscribe: stores.navigating.subscribe
				};
			},
			enumerable: false
		},
		session: {
			get() {
				removed_session();
				return {};
			},
			enumerable: false
		}
	});

	return readonly_stores;
};

/** @type {typeof import('$app/stores').page} */
const page = {
	/** @param {(value: any) => void} fn */
	subscribe(fn) {
		const store = getStores().page;
		return store.subscribe(fn);
	}
};

function removed_session() {
	// TODO remove for 1.0
	throw new Error(
		'stores.session is no longer available. See https://github.com/sveltejs/kit/discussions/5883'
	);
}

/* C:/dev/git/IronRDP/iron-svelte-client/node_modules/@sveltejs/kit/src/runtime/components/error.svelte generated by Svelte v3.53.1 */

const Error$1 = create_ssr_component(($$result, $$props, $$bindings, slots) => {
	let $page, $$unsubscribe_page;
	$$unsubscribe_page = subscribe(page, value => $page = value);
	$$unsubscribe_page();

	return `<h1>${escape($page.status)}</h1>

<pre>${escape($page.error.message)}</pre>



${$page.error.frame
	? `<pre>${escape($page.error.frame)}</pre>`
	: ``}
${$page.error.stack
	? `<pre>${escape($page.error.stack)}</pre>`
	: ``}`;
});

export { Error$1 as default };
