import { c as create_ssr_component, d as add_attribute, e as escape, v as validate_component } from "../../../chunks/index.js";
let Page;
let __tla = (async () => {
  class LoginPage {
    username = "";
    password = "";
    host = "";
    toastText = "";
  }
  ui;
  const Login = new LoginPage();
  let serverBridge;
  {
    import("../../../chunks/wasm-bridge.service.js").then(async (m) => {
      await m.__tla;
      return m;
    }).then((module) => {
      serverBridge = new module.WasmBridgeService();
      serverBridge.init();
    });
  }
  const login_svelte_svelte_type_style_lang = "";
  const css = {
    code: "@import './login.css';",
    map: null
  };
  const Login_1 = create_ssr_component(($$result, $$props, $$bindings, slots) => {
    $$result.css.add(css);
    return `<main class="${"responsive"}"><div class="${"large-space"}"></div>
	<div class="${"grid"}"><div class="${"s2"}"></div>
		<div class="${"s8"}"><article class="${"primary-container"}"><h5>Login</h5>
				<div class="${"medium-space"}"></div>
				<div><div class="${"field label border"}"><input id="${"host"}" type="${"text"}"${add_attribute("value", Login.host, 0)}>
						<label for="${"host"}">Host</label></div>
					<div class="${"field label border"}"><input id="${"username"}" type="${"text"}"${add_attribute("value", Login.username, 0)}>
						<label for="${"username"}">Username</label></div>
					<div class="${"field label border"}"><input id="${"password"}" type="${"password"}"${add_attribute("value", Login.password, 0)}>
						<label for="${"password"}">Password</label></div></div>
				<nav class="${"center-align"}"><button>Login</button></nav></article></div>
		<div class="${"s2"}"></div></div></main>

<div id="${"toast"}" class="${"toast blue white-text"}"><i>info</i>
	<span>${escape(Login.toastText)}</span>
</div>`;
  });
  Page = create_ssr_component(($$result, $$props, $$bindings, slots) => {
    return `${validate_component(Login_1, "Login").$$render($$result, {}, {}, {})}`;
  });
})();
export {
  __tla,
  Page as default
};
