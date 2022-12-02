import { c as create_ssr_component, s as setContext, v as validate_component, m as missing_component, n as noop, a as safe_not_equal } from "./chunks/index.js";
import * as devalue from "devalue";
import { parse, serialize } from "cookie";
import * as set_cookie_parser from "set-cookie-parser";
let Server, override;
let __tla = (async () => {
  function afterUpdate() {
  }
  function set_building(value) {
  }
  const Root = create_ssr_component(($$result, $$props, $$bindings, slots) => {
    let { stores } = $$props;
    let { page } = $$props;
    let { components } = $$props;
    let { form } = $$props;
    let { data_0 = null } = $$props;
    let { data_1 = null } = $$props;
    {
      setContext("__svelte__", stores);
    }
    afterUpdate(stores.page.notify);
    if ($$props.stores === void 0 && $$bindings.stores && stores !== void 0)
      $$bindings.stores(stores);
    if ($$props.page === void 0 && $$bindings.page && page !== void 0)
      $$bindings.page(page);
    if ($$props.components === void 0 && $$bindings.components && components !== void 0)
      $$bindings.components(components);
    if ($$props.form === void 0 && $$bindings.form && form !== void 0)
      $$bindings.form(form);
    if ($$props.data_0 === void 0 && $$bindings.data_0 && data_0 !== void 0)
      $$bindings.data_0(data_0);
    if ($$props.data_1 === void 0 && $$bindings.data_1 && data_1 !== void 0)
      $$bindings.data_1(data_1);
    {
      stores.page.set(page);
    }
    return `


${components[1] ? `${validate_component(components[0] || missing_component, "svelte:component").$$render($$result, {
      data: data_0
    }, {}, {
      default: () => {
        return `${validate_component(components[1] || missing_component, "svelte:component").$$render($$result, {
          data: data_1,
          form
        }, {}, {})}`;
      }
    })}` : `${validate_component(components[0] || missing_component, "svelte:component").$$render($$result, {
      data: data_0,
      form
    }, {}, {})}`}

${``}`;
  });
  function negotiate(accept, types) {
    const parts = [];
    accept.split(",").forEach((str, i) => {
      const match = /([^/]+)\/([^;]+)(?:;q=([0-9.]+))?/.exec(str);
      if (match) {
        const [, type, subtype, q = "1"] = match;
        parts.push({
          type,
          subtype,
          q: +q,
          i
        });
      }
    });
    parts.sort((a, b) => {
      if (a.q !== b.q) {
        return b.q - a.q;
      }
      if (a.subtype === "*" !== (b.subtype === "*")) {
        return a.subtype === "*" ? 1 : -1;
      }
      if (a.type === "*" !== (b.type === "*")) {
        return a.type === "*" ? 1 : -1;
      }
      return a.i - b.i;
    });
    let accepted;
    let min_priority = Infinity;
    for (const mimetype of types) {
      const [type, subtype] = mimetype.split("/");
      const priority = parts.findIndex((part) => (part.type === type || part.type === "*") && (part.subtype === subtype || part.subtype === "*"));
      if (priority !== -1 && priority < min_priority) {
        accepted = mimetype;
        min_priority = priority;
      }
    }
    return accepted;
  }
  function is_content_type(request, ...types) {
    var _a;
    const type = ((_a = request.headers.get("content-type")) == null ? void 0 : _a.split(";", 1)[0].trim()) ?? "";
    return types.includes(type);
  }
  function is_form_content_type(request) {
    return is_content_type(request, "application/x-www-form-urlencoded", "multipart/form-data");
  }
  class HttpError {
    constructor(status, body) {
      this.status = status;
      if (typeof body === "string") {
        this.body = {
          message: body
        };
      } else if (body) {
        this.body = body;
      } else {
        this.body = {
          message: `Error: ${status}`
        };
      }
    }
    toString() {
      return JSON.stringify(this.body);
    }
  }
  class Redirect {
    constructor(status, location) {
      this.status = status;
      this.location = location;
    }
  }
  class ValidationError {
    constructor(status, data) {
      this.status = status;
      this.data = data;
    }
  }
  function coalesce_to_error(err) {
    return err instanceof Error || err && err.name && err.message ? err : new Error(JSON.stringify(err));
  }
  function normalize_error(error2) {
    return error2;
  }
  function normalize_path(path, trailing_slash) {
    if (path === "/" || trailing_slash === "ignore")
      return path;
    if (trailing_slash === "never") {
      return path.endsWith("/") ? path.slice(0, -1) : path;
    } else if (trailing_slash === "always" && !path.endsWith("/")) {
      return path + "/";
    }
    return path;
  }
  function decode_pathname(pathname) {
    return pathname.split("%25").map(decodeURI).join("%25");
  }
  function decode_params(params) {
    for (const key2 in params) {
      params[key2] = decodeURIComponent(params[key2]);
    }
    return params;
  }
  const tracked_url_properties = [
    "href",
    "pathname",
    "search",
    "searchParams",
    "toString",
    "toJSON"
  ];
  function make_trackable(url, callback) {
    const tracked = new URL(url);
    for (const property of tracked_url_properties) {
      let value = tracked[property];
      Object.defineProperty(tracked, property, {
        get() {
          callback();
          return value;
        },
        enumerable: true,
        configurable: true
      });
    }
    {
      tracked[Symbol.for("nodejs.util.inspect.custom")] = (depth, opts, inspect) => {
        return inspect(url, opts);
      };
    }
    disable_hash(tracked);
    return tracked;
  }
  function disable_hash(url) {
    Object.defineProperty(url, "hash", {
      get() {
        throw new Error("Cannot access event.url.hash. Consider using `$page.url.hash` inside a component instead");
      }
    });
  }
  function disable_search(url) {
    for (const property of [
      "search",
      "searchParams"
    ]) {
      Object.defineProperty(url, property, {
        get() {
          throw new Error(`Cannot access url.${property} on a page with prerendering enabled`);
        }
      });
    }
  }
  const DATA_SUFFIX = "/__data.json";
  function has_data_suffix(pathname) {
    return pathname.endsWith(DATA_SUFFIX);
  }
  function add_data_suffix(pathname) {
    return pathname.replace(/\/$/, "") + DATA_SUFFIX;
  }
  function strip_data_suffix(pathname) {
    return pathname.slice(0, -DATA_SUFFIX.length);
  }
  function check_method_names(mod) {
    [
      "get",
      "post",
      "put",
      "patch",
      "del"
    ].forEach((m) => {
      if (m in mod) {
        const replacement = m === "del" ? "DELETE" : m.toUpperCase();
        throw Error(`Endpoint method "${m}" has changed to "${replacement}". See https://github.com/sveltejs/kit/discussions/5359 for more information.`);
      }
    });
  }
  const GENERIC_ERROR = {
    id: "__error"
  };
  function method_not_allowed(mod, method) {
    return new Response(`${method} method not allowed`, {
      status: 405,
      headers: {
        allow: allowed_methods(mod).join(", ")
      }
    });
  }
  function allowed_methods(mod) {
    const allowed = [];
    for (const method in [
      "GET",
      "POST",
      "PUT",
      "PATCH",
      "DELETE"
    ]) {
      if (method in mod)
        allowed.push(method);
    }
    if (mod.GET || mod.HEAD)
      allowed.push("HEAD");
    return allowed;
  }
  function get_option(nodes, option) {
    return nodes.reduce((value, node) => {
      var _a, _b;
      for (const thing of [
        node == null ? void 0 : node.server,
        node == null ? void 0 : node.shared
      ]) {
        if (thing && ("router" in thing || "hydrate" in thing)) {
          throw new Error("`export const hydrate` and `export const router` have been replaced with `export const csr`. See https://github.com/sveltejs/kit/pull/6446");
        }
      }
      return ((_a = node == null ? void 0 : node.shared) == null ? void 0 : _a[option]) ?? ((_b = node == null ? void 0 : node.server) == null ? void 0 : _b[option]) ?? value;
    }, void 0);
  }
  function static_error_page(options, status, message) {
    return new Response(options.error_template({
      status,
      message
    }), {
      headers: {
        "content-type": "text/html; charset=utf-8"
      },
      status
    });
  }
  function handle_fatal_error(event, options, error2) {
    error2 = error2 instanceof HttpError ? error2 : coalesce_to_error(error2);
    const status = error2 instanceof HttpError ? error2.status : 500;
    const body = handle_error_and_jsonify(event, options, error2);
    const type = negotiate(event.request.headers.get("accept") || "text/html", [
      "application/json",
      "text/html"
    ]);
    if (has_data_suffix(event.url.pathname) || type === "application/json") {
      return new Response(JSON.stringify(body), {
        status,
        headers: {
          "content-type": "application/json; charset=utf-8"
        }
      });
    }
    return static_error_page(options, status, body.message);
  }
  function handle_error_and_jsonify(event, options, error2) {
    if (error2 instanceof HttpError) {
      return error2.body;
    } else {
      return options.handle_error(error2, event);
    }
  }
  function redirect_response(status, location) {
    const response = new Response(void 0, {
      status,
      headers: {
        location
      }
    });
    return response;
  }
  function clarify_devalue_error(event, error2) {
    if (error2.path) {
      return `Data returned from \`load\` while rendering ${event.route.id} is not serializable: ${error2.message} (data${error2.path})`;
    }
    if (error2.path === "") {
      return `Data returned from \`load\` while rendering ${event.route.id} is not a plain object`;
    }
    return error2.message;
  }
  function serialize_data_node(node) {
    if (!node)
      return "null";
    if (node.type === "error" || node.type === "skip") {
      return JSON.stringify(node);
    }
    const stringified = devalue.stringify(node.data);
    const uses = [];
    if (node.uses.dependencies.size > 0) {
      uses.push(`"dependencies":${JSON.stringify(Array.from(node.uses.dependencies))}`);
    }
    if (node.uses.params.size > 0) {
      uses.push(`"params":${JSON.stringify(Array.from(node.uses.params))}`);
    }
    if (node.uses.parent)
      uses.push(`"parent":1`);
    if (node.uses.route)
      uses.push(`"route":1`);
    if (node.uses.url)
      uses.push(`"url":1`);
    return `{"type":"data","data":${stringified},"uses":{${uses.join(",")}}${node.slash ? `,"slash":${JSON.stringify(node.slash)}` : ""}}`;
  }
  async function render_endpoint(event, mod, state) {
    const method = event.request.method;
    check_method_names(mod);
    let handler = mod[method];
    if (!handler && method === "HEAD") {
      handler = mod.GET;
    }
    if (!handler) {
      return method_not_allowed(mod, method);
    }
    const prerender = mod.prerender ?? state.prerender_default;
    if (prerender && (mod.POST || mod.PATCH || mod.PUT || mod.DELETE)) {
      throw new Error("Cannot prerender endpoints that have mutative methods");
    }
    if (state.prerendering && !prerender) {
      if (state.initiator) {
        throw new Error(`${event.route.id} is not prerenderable`);
      } else {
        return new Response(void 0, {
          status: 204
        });
      }
    }
    try {
      const response = await handler(event);
      if (!(response instanceof Response)) {
        throw new Error(`Invalid response from route ${event.url.pathname}: handler should return a Response object`);
      }
      if (state.prerendering) {
        response.headers.set("x-sveltekit-prerender", String(prerender));
      }
      return response;
    } catch (error2) {
      if (error2 instanceof Redirect) {
        return new Response(void 0, {
          status: error2.status,
          headers: {
            location: error2.location
          }
        });
      }
      throw error2;
    }
  }
  function is_endpoint_request(event) {
    const { method, headers } = event.request;
    if (method === "PUT" || method === "PATCH" || method === "DELETE") {
      return true;
    }
    if (method === "POST" && headers.get("x-sveltekit-action") === "true")
      return false;
    const accept = event.request.headers.get("accept") ?? "*/*";
    return negotiate(accept, [
      "*",
      "text/html"
    ]) !== "text/html";
  }
  function compact(arr) {
    return arr.filter((val) => val != null);
  }
  function error(status, message) {
    if (isNaN(status) || status < 400 || status > 599) {
      throw new Error(`HTTP error status codes must be between 400 and 599 \u2014 ${status} is invalid`);
    }
    return new HttpError(status, message);
  }
  function json(data, init2) {
    const headers = new Headers(init2 == null ? void 0 : init2.headers);
    if (!headers.has("content-type")) {
      headers.set("content-type", "application/json");
    }
    return new Response(JSON.stringify(data), {
      ...init2,
      headers
    });
  }
  function is_action_json_request(event) {
    const accept = negotiate(event.request.headers.get("accept") ?? "*/*", [
      "application/json",
      "text/html"
    ]);
    return accept === "application/json" && event.request.method === "POST";
  }
  async function handle_action_json_request(event, options, server) {
    const actions = server.actions;
    if (!actions) {
      maybe_throw_migration_error(server);
      return new Response("POST method not allowed. No actions exist for this page", {
        status: 405,
        headers: {
          allow: "GET"
        }
      });
    }
    check_named_default_separate(actions);
    try {
      const data = await call_action(event, actions);
      if (data instanceof ValidationError) {
        return action_json({
          type: "invalid",
          status: data.status,
          data: stringify_action_response(data.data, event.route.id)
        });
      } else {
        return action_json({
          type: "success",
          status: data ? 200 : 204,
          data: stringify_action_response(data, event.route.id)
        });
      }
    } catch (e) {
      const error2 = normalize_error(e);
      if (error2 instanceof Redirect) {
        return action_json({
          type: "redirect",
          status: error2.status,
          location: error2.location
        });
      }
      return action_json({
        type: "error",
        error: handle_error_and_jsonify(event, options, check_incorrect_invalid_use(error2))
      }, {
        status: error2 instanceof HttpError ? error2.status : 500
      });
    }
  }
  function check_incorrect_invalid_use(error2) {
    return error2 instanceof ValidationError ? new Error(`Cannot "throw invalid()". Use "return invalid()"`) : error2;
  }
  function action_json(data, init2) {
    return json(data, init2);
  }
  function is_action_request(event, leaf_node) {
    return leaf_node.server && event.request.method !== "GET" && event.request.method !== "HEAD";
  }
  async function handle_action_request(event, server) {
    const actions = server.actions;
    if (!actions) {
      maybe_throw_migration_error(server);
      event.setHeaders({
        allow: "GET"
      });
      return {
        type: "error",
        error: error(405, "POST method not allowed. No actions exist for this page")
      };
    }
    check_named_default_separate(actions);
    try {
      const data = await call_action(event, actions);
      if (data instanceof ValidationError) {
        return {
          type: "invalid",
          status: data.status,
          data: data.data
        };
      } else {
        return {
          type: "success",
          status: 200,
          data
        };
      }
    } catch (e) {
      const error2 = normalize_error(e);
      if (error2 instanceof Redirect) {
        return {
          type: "redirect",
          status: error2.status,
          location: error2.location
        };
      }
      return {
        type: "error",
        error: check_incorrect_invalid_use(error2)
      };
    }
  }
  function check_named_default_separate(actions) {
    if (actions.default && Object.keys(actions).length > 1) {
      throw new Error(`When using named actions, the default action cannot be used. See the docs for more info: https://kit.svelte.dev/docs/form-actions#named-actions`);
    }
  }
  async function call_action(event, actions) {
    const url = new URL(event.request.url);
    let name = "default";
    for (const param of url.searchParams) {
      if (param[0].startsWith("/")) {
        name = param[0].slice(1);
        if (name === "default") {
          throw new Error('Cannot use reserved action name "default"');
        }
        break;
      }
    }
    const action = actions[name];
    if (!action) {
      throw new Error(`No action with name '${name}' found`);
    }
    if (!is_form_content_type(event.request)) {
      throw new Error(`Actions expect form-encoded data (received ${event.request.headers.get("content-type")}`);
    }
    return action(event);
  }
  function maybe_throw_migration_error(server) {
    for (const method of [
      "POST",
      "PUT",
      "PATCH",
      "DELETE"
    ]) {
      if (server[method]) {
        throw new Error(`${method} method no longer allowed in +page.server, use actions instead. See the PR for more info: https://github.com/sveltejs/kit/pull/6469`);
      }
    }
  }
  function uneval_action_response(data, route_id) {
    return try_deserialize(data, devalue.uneval, route_id);
  }
  function stringify_action_response(data, route_id) {
    return try_deserialize(data, devalue.stringify, route_id);
  }
  function try_deserialize(data, fn, route_id) {
    try {
      return fn(data);
    } catch (e) {
      const error2 = e;
      if ("path" in error2) {
        let message = `Data returned from action inside ${route_id} is not serializable: ${error2.message}`;
        if (error2.path !== "")
          message += ` (data.${error2.path})`;
        throw new Error(message);
      }
      throw error2;
    }
  }
  async function unwrap_promises(object) {
    var _a;
    for (const key2 in object) {
      if (typeof ((_a = object[key2]) == null ? void 0 : _a.then) === "function") {
        return Object.fromEntries(await Promise.all(Object.entries(object).map(async ([key3, value]) => [
          key3,
          await value
        ])));
      }
    }
    return object;
  }
  async function load_server_data({ event, state, node, parent }) {
    var _a;
    if (!(node == null ? void 0 : node.server))
      return null;
    const uses = {
      dependencies: /* @__PURE__ */ new Set(),
      params: /* @__PURE__ */ new Set(),
      parent: false,
      route: false,
      url: false
    };
    const url = make_trackable(event.url, () => {
      uses.url = true;
    });
    if (state.prerendering) {
      disable_search(url);
    }
    const result = await ((_a = node.server.load) == null ? void 0 : _a.call(null, {
      ...event,
      depends: (...deps) => {
        for (const dep of deps) {
          const { href } = new URL(dep, event.url);
          uses.dependencies.add(href);
        }
      },
      params: new Proxy(event.params, {
        get: (target, key2) => {
          uses.params.add(key2);
          return target[key2];
        }
      }),
      parent: async () => {
        uses.parent = true;
        return parent();
      },
      route: {
        get id() {
          uses.route = true;
          return event.route.id;
        }
      },
      url
    }));
    const data = result ? await unwrap_promises(result) : null;
    return {
      type: "data",
      data,
      uses,
      slash: node.server.trailingSlash
    };
  }
  async function load_data({ event, fetched, node, parent, server_data_promise, state, resolve_opts, csr }) {
    var _a;
    const server_data_node = await server_data_promise;
    if (!((_a = node == null ? void 0 : node.shared) == null ? void 0 : _a.load)) {
      return (server_data_node == null ? void 0 : server_data_node.data) ?? null;
    }
    const load_event = {
      url: event.url,
      params: event.params,
      data: (server_data_node == null ? void 0 : server_data_node.data) ?? null,
      route: event.route,
      fetch: async (input, init2) => {
        const cloned_body = input instanceof Request && input.body ? input.clone().body : null;
        const response = await event.fetch(input, init2);
        const url = new URL(input instanceof Request ? input.url : input, event.url);
        const same_origin = url.origin === event.url.origin;
        let dependency;
        if (same_origin) {
          if (state.prerendering) {
            dependency = {
              response,
              body: null
            };
            state.prerendering.dependencies.set(url.pathname, dependency);
          }
        } else {
          const mode = input instanceof Request ? input.mode : (init2 == null ? void 0 : init2.mode) ?? "cors";
          if (mode !== "no-cors") {
            const acao = response.headers.get("access-control-allow-origin");
            if (!acao || acao !== event.url.origin && acao !== "*") {
              throw new Error(`CORS error: ${acao ? "Incorrect" : "No"} 'Access-Control-Allow-Origin' header is present on the requested resource`);
            }
          }
        }
        const proxy = new Proxy(response, {
          get(response2, key2, _receiver) {
            async function text() {
              const body = await response2.text();
              if (!body || typeof body === "string") {
                const status_number = Number(response2.status);
                if (isNaN(status_number)) {
                  throw new Error(`response.status is not a number. value: "${response2.status}" type: ${typeof response2.status}`);
                }
                fetched.push({
                  url: same_origin ? url.href.slice(event.url.origin.length) : url.href,
                  method: event.request.method,
                  request_body: input instanceof Request && cloned_body ? await stream_to_string(cloned_body) : init2 == null ? void 0 : init2.body,
                  response_body: body,
                  response: response2
                });
              }
              if (dependency) {
                dependency.body = body;
              }
              return body;
            }
            if (key2 === "arrayBuffer") {
              return async () => {
                const buffer = await response2.arrayBuffer();
                if (dependency) {
                  dependency.body = new Uint8Array(buffer);
                }
                return buffer;
              };
            }
            if (key2 === "text") {
              return text;
            }
            if (key2 === "json") {
              return async () => {
                return JSON.parse(await text());
              };
            }
            return Reflect.get(response2, key2, response2);
          }
        });
        if (csr) {
          const get = response.headers.get;
          response.headers.get = (key2) => {
            const lower = key2.toLowerCase();
            const value = get.call(response.headers, lower);
            if (value && !lower.startsWith("x-sveltekit-")) {
              const included = resolve_opts.filterSerializedResponseHeaders(lower, value);
              if (!included) {
                throw new Error(`Failed to get response header "${lower}" \u2014 it must be included by the \`filterSerializedResponseHeaders\` option: https://kit.svelte.dev/docs/hooks#server-hooks-handle (at ${event.route})`);
              }
            }
            return value;
          };
        }
        return proxy;
      },
      setHeaders: event.setHeaders,
      depends: () => {
      },
      parent
    };
    Object.defineProperties(load_event, {
      session: {
        get() {
          throw new Error("session is no longer available. See https://github.com/sveltejs/kit/discussions/5883");
        },
        enumerable: false
      }
    });
    const data = await node.shared.load.call(null, load_event);
    return data ? unwrap_promises(data) : null;
  }
  async function stream_to_string(stream) {
    let result = "";
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      result += decoder.decode(value);
    }
    return result;
  }
  const subscriber_queue = [];
  function readable(value, start) {
    return {
      subscribe: writable(value, start).subscribe
    };
  }
  function writable(value, start = noop) {
    let stop;
    const subscribers = /* @__PURE__ */ new Set();
    function set(new_value) {
      if (safe_not_equal(value, new_value)) {
        value = new_value;
        if (stop) {
          const run_queue = !subscriber_queue.length;
          for (const subscriber of subscribers) {
            subscriber[1]();
            subscriber_queue.push(subscriber, value);
          }
          if (run_queue) {
            for (let i = 0; i < subscriber_queue.length; i += 2) {
              subscriber_queue[i][0](subscriber_queue[i + 1]);
            }
            subscriber_queue.length = 0;
          }
        }
      }
    }
    function update(fn) {
      set(fn(value));
    }
    function subscribe(run, invalidate = noop) {
      const subscriber = [
        run,
        invalidate
      ];
      subscribers.add(subscriber);
      if (subscribers.size === 1) {
        stop = start(set) || noop;
      }
      run(value);
      return () => {
        subscribers.delete(subscriber);
        if (subscribers.size === 0) {
          stop();
          stop = null;
        }
      };
    }
    return {
      set,
      update,
      subscribe
    };
  }
  function hash(value) {
    let hash2 = 5381;
    if (typeof value === "string") {
      let i = value.length;
      while (i)
        hash2 = hash2 * 33 ^ value.charCodeAt(--i);
    } else if (ArrayBuffer.isView(value)) {
      const buffer = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      let i = buffer.length;
      while (i)
        hash2 = hash2 * 33 ^ buffer[--i];
    } else {
      throw new TypeError("value must be a string or TypedArray");
    }
    return (hash2 >>> 0).toString(36);
  }
  const escape_html_attr_dict = {
    "&": "&amp;",
    '"': "&quot;"
  };
  const escape_html_attr_regex = new RegExp(`[${Object.keys(escape_html_attr_dict).join("")}]|[\uD800-\uDBFF](?![\uDC00-\uDFFF])|[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uDC00-\uDFFF]`, "g");
  function escape_html_attr(str) {
    const escaped_str = str.replace(escape_html_attr_regex, (match) => {
      if (match.length === 2) {
        return match;
      }
      return escape_html_attr_dict[match] ?? `&#${match.charCodeAt(0)};`;
    });
    return `"${escaped_str}"`;
  }
  const replacements = {
    "<": "\\u003C",
    "\u2028": "\\u2028",
    "\u2029": "\\u2029"
  };
  const pattern = new RegExp(`[${Object.keys(replacements).join("")}]`, "g");
  function serialize_data(fetched, filter, prerendering = false) {
    const headers = {};
    let cache_control = null;
    let age = null;
    for (const [key2, value] of fetched.response.headers) {
      if (filter(key2, value)) {
        headers[key2] = value;
      }
      if (key2 === "cache-control")
        cache_control = value;
      if (key2 === "age")
        age = value;
    }
    const payload = {
      status: fetched.response.status,
      statusText: fetched.response.statusText,
      headers,
      body: fetched.response_body
    };
    const safe_payload = JSON.stringify(payload).replace(pattern, (match) => replacements[match]);
    const attrs = [
      'type="application/json"',
      "data-sveltekit-fetched",
      `data-url=${escape_html_attr(fetched.url)}`
    ];
    if (fetched.request_body) {
      attrs.push(`data-hash=${escape_html_attr(hash(fetched.request_body))}`);
    }
    if (!prerendering && fetched.method === "GET" && cache_control) {
      const match = /s-maxage=(\d+)/g.exec(cache_control) ?? /max-age=(\d+)/g.exec(cache_control);
      if (match) {
        const ttl = +match[1] - +(age ?? "0");
        attrs.push(`data-ttl="${ttl}"`);
      }
    }
    return `<script ${attrs.join(" ")}>${safe_payload}<\/script>`;
  }
  const s = JSON.stringify;
  const encoder = new TextEncoder();
  function sha256(data) {
    if (!key[0])
      precompute();
    const out = init.slice(0);
    const array2 = encode$1(data);
    for (let i = 0; i < array2.length; i += 16) {
      const w = array2.subarray(i, i + 16);
      let tmp;
      let a;
      let b;
      let out0 = out[0];
      let out1 = out[1];
      let out2 = out[2];
      let out3 = out[3];
      let out4 = out[4];
      let out5 = out[5];
      let out6 = out[6];
      let out7 = out[7];
      for (let i2 = 0; i2 < 64; i2++) {
        if (i2 < 16) {
          tmp = w[i2];
        } else {
          a = w[i2 + 1 & 15];
          b = w[i2 + 14 & 15];
          tmp = w[i2 & 15] = (a >>> 7 ^ a >>> 18 ^ a >>> 3 ^ a << 25 ^ a << 14) + (b >>> 17 ^ b >>> 19 ^ b >>> 10 ^ b << 15 ^ b << 13) + w[i2 & 15] + w[i2 + 9 & 15] | 0;
        }
        tmp = tmp + out7 + (out4 >>> 6 ^ out4 >>> 11 ^ out4 >>> 25 ^ out4 << 26 ^ out4 << 21 ^ out4 << 7) + (out6 ^ out4 & (out5 ^ out6)) + key[i2];
        out7 = out6;
        out6 = out5;
        out5 = out4;
        out4 = out3 + tmp | 0;
        out3 = out2;
        out2 = out1;
        out1 = out0;
        out0 = tmp + (out1 & out2 ^ out3 & (out1 ^ out2)) + (out1 >>> 2 ^ out1 >>> 13 ^ out1 >>> 22 ^ out1 << 30 ^ out1 << 19 ^ out1 << 10) | 0;
      }
      out[0] = out[0] + out0 | 0;
      out[1] = out[1] + out1 | 0;
      out[2] = out[2] + out2 | 0;
      out[3] = out[3] + out3 | 0;
      out[4] = out[4] + out4 | 0;
      out[5] = out[5] + out5 | 0;
      out[6] = out[6] + out6 | 0;
      out[7] = out[7] + out7 | 0;
    }
    const bytes = new Uint8Array(out.buffer);
    reverse_endianness(bytes);
    return base64(bytes);
  }
  const init = new Uint32Array(8);
  const key = new Uint32Array(64);
  function precompute() {
    function frac(x) {
      return (x - Math.floor(x)) * 4294967296;
    }
    let prime = 2;
    for (let i = 0; i < 64; prime++) {
      let is_prime = true;
      for (let factor = 2; factor * factor <= prime; factor++) {
        if (prime % factor === 0) {
          is_prime = false;
          break;
        }
      }
      if (is_prime) {
        if (i < 8) {
          init[i] = frac(prime ** (1 / 2));
        }
        key[i] = frac(prime ** (1 / 3));
        i++;
      }
    }
  }
  function reverse_endianness(bytes) {
    for (let i = 0; i < bytes.length; i += 4) {
      const a = bytes[i + 0];
      const b = bytes[i + 1];
      const c = bytes[i + 2];
      const d = bytes[i + 3];
      bytes[i + 0] = d;
      bytes[i + 1] = c;
      bytes[i + 2] = b;
      bytes[i + 3] = a;
    }
  }
  function encode$1(str) {
    const encoded = encoder.encode(str);
    const length = encoded.length * 8;
    const size = 512 * Math.ceil((length + 65) / 512);
    const bytes = new Uint8Array(size / 8);
    bytes.set(encoded);
    bytes[encoded.length] = 128;
    reverse_endianness(bytes);
    const words = new Uint32Array(bytes.buffer);
    words[words.length - 2] = Math.floor(length / 4294967296);
    words[words.length - 1] = length;
    return words;
  }
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".split("");
  function base64(bytes) {
    const l = bytes.length;
    let result = "";
    let i;
    for (i = 2; i < l; i += 3) {
      result += chars[bytes[i - 2] >> 2];
      result += chars[(bytes[i - 2] & 3) << 4 | bytes[i - 1] >> 4];
      result += chars[(bytes[i - 1] & 15) << 2 | bytes[i] >> 6];
      result += chars[bytes[i] & 63];
    }
    if (i === l + 1) {
      result += chars[bytes[i - 2] >> 2];
      result += chars[(bytes[i - 2] & 3) << 4];
      result += "==";
    }
    if (i === l) {
      result += chars[bytes[i - 2] >> 2];
      result += chars[(bytes[i - 2] & 3) << 4 | bytes[i - 1] >> 4];
      result += chars[(bytes[i - 1] & 15) << 2];
      result += "=";
    }
    return result;
  }
  const array = new Uint8Array(16);
  function generate_nonce() {
    crypto.getRandomValues(array);
    return base64(array);
  }
  const quoted = /* @__PURE__ */ new Set([
    "self",
    "unsafe-eval",
    "unsafe-hashes",
    "unsafe-inline",
    "none",
    "strict-dynamic",
    "report-sample",
    "wasm-unsafe-eval"
  ]);
  const crypto_pattern = /^(nonce|sha\d\d\d)-/;
  class BaseProvider {
    #use_hashes;
    #script_needs_csp;
    #style_needs_csp;
    #directives;
    #script_src;
    #style_src;
    #nonce;
    constructor(use_hashes, directives, nonce, dev) {
      this.#use_hashes = use_hashes;
      this.#directives = dev ? {
        ...directives
      } : directives;
      const d = this.#directives;
      if (dev) {
        const effective_style_src2 = d["style-src"] || d["default-src"];
        if (effective_style_src2 && !effective_style_src2.includes("unsafe-inline")) {
          d["style-src"] = [
            ...effective_style_src2,
            "unsafe-inline"
          ];
        }
      }
      this.#script_src = [];
      this.#style_src = [];
      const effective_script_src = d["script-src"] || d["default-src"];
      const effective_style_src = d["style-src"] || d["default-src"];
      this.#script_needs_csp = !!effective_script_src && effective_script_src.filter((value) => value !== "unsafe-inline").length > 0;
      this.#style_needs_csp = !dev && !!effective_style_src && effective_style_src.filter((value) => value !== "unsafe-inline").length > 0;
      this.script_needs_nonce = this.#script_needs_csp && !this.#use_hashes;
      this.style_needs_nonce = this.#style_needs_csp && !this.#use_hashes;
      this.#nonce = nonce;
    }
    add_script(content) {
      if (this.#script_needs_csp) {
        if (this.#use_hashes) {
          this.#script_src.push(`sha256-${sha256(content)}`);
        } else if (this.#script_src.length === 0) {
          this.#script_src.push(`nonce-${this.#nonce}`);
        }
      }
    }
    add_style(content) {
      if (this.#style_needs_csp) {
        if (this.#use_hashes) {
          this.#style_src.push(`sha256-${sha256(content)}`);
        } else if (this.#style_src.length === 0) {
          this.#style_src.push(`nonce-${this.#nonce}`);
        }
      }
    }
    get_header(is_meta = false) {
      const header = [];
      const directives = {
        ...this.#directives
      };
      if (this.#style_src.length > 0) {
        directives["style-src"] = [
          ...directives["style-src"] || directives["default-src"] || [],
          ...this.#style_src
        ];
      }
      if (this.#script_src.length > 0) {
        directives["script-src"] = [
          ...directives["script-src"] || directives["default-src"] || [],
          ...this.#script_src
        ];
      }
      for (const key2 in directives) {
        if (is_meta && (key2 === "frame-ancestors" || key2 === "report-uri" || key2 === "sandbox")) {
          continue;
        }
        const value = directives[key2];
        if (!value)
          continue;
        const directive = [
          key2
        ];
        if (Array.isArray(value)) {
          value.forEach((value2) => {
            if (quoted.has(value2) || crypto_pattern.test(value2)) {
              directive.push(`'${value2}'`);
            } else {
              directive.push(value2);
            }
          });
        }
        header.push(directive.join(" "));
      }
      return header.join("; ");
    }
  }
  class CspProvider extends BaseProvider {
    get_meta() {
      const content = escape_html_attr(this.get_header(true));
      return `<meta http-equiv="content-security-policy" content=${content}>`;
    }
  }
  class CspReportOnlyProvider extends BaseProvider {
    constructor(use_hashes, directives, nonce, dev) {
      var _a, _b;
      super(use_hashes, directives, nonce, dev);
      if (Object.values(directives).filter((v) => !!v).length > 0) {
        const has_report_to = ((_a = directives["report-to"]) == null ? void 0 : _a.length) ?? 0 > 0;
        const has_report_uri = ((_b = directives["report-uri"]) == null ? void 0 : _b.length) ?? 0 > 0;
        if (!has_report_to && !has_report_uri) {
          throw Error("`content-security-policy-report-only` must be specified with either the `report-to` or `report-uri` directives, or both");
        }
      }
    }
  }
  class Csp {
    nonce = generate_nonce();
    csp_provider;
    report_only_provider;
    constructor({ mode, directives, reportOnly }, { prerender, dev }) {
      const use_hashes = mode === "hash" || mode === "auto" && prerender;
      this.csp_provider = new CspProvider(use_hashes, directives, this.nonce, dev);
      this.report_only_provider = new CspReportOnlyProvider(use_hashes, reportOnly, this.nonce, dev);
    }
    get script_needs_nonce() {
      return this.csp_provider.script_needs_nonce || this.report_only_provider.script_needs_nonce;
    }
    get style_needs_nonce() {
      return this.csp_provider.style_needs_nonce || this.report_only_provider.style_needs_nonce;
    }
    add_script(content) {
      this.csp_provider.add_script(content);
      this.report_only_provider.add_script(content);
    }
    add_style(content) {
      this.csp_provider.add_style(content);
      this.report_only_provider.add_style(content);
    }
  }
  const updated = {
    ...readable(false),
    check: () => false
  };
  async function render_response({ branch, fetched, options, state, page_config, status, error: error2 = null, event, resolve_opts, action_result }) {
    var _a;
    if (state.prerendering) {
      if (options.csp.mode === "nonce") {
        throw new Error('Cannot use prerendering if config.kit.csp.mode === "nonce"');
      }
      if (options.app_template_contains_nonce) {
        throw new Error("Cannot use prerendering if page template contains %sveltekit.nonce%");
      }
    }
    const { entry } = options.manifest._;
    const stylesheets = new Set(entry.stylesheets);
    const modulepreloads = new Set(entry.imports);
    const fonts = new Set(options.manifest._.entry.fonts);
    const link_header_preloads = /* @__PURE__ */ new Set();
    const inline_styles = /* @__PURE__ */ new Map();
    let rendered;
    const form_value = (action_result == null ? void 0 : action_result.type) === "success" || (action_result == null ? void 0 : action_result.type) === "invalid" ? action_result.data ?? null : null;
    if (page_config.ssr) {
      const props = {
        stores: {
          page: writable(null),
          navigating: writable(null),
          updated
        },
        components: await Promise.all(branch.map(({ node }) => node.component())),
        form: form_value
      };
      let data = {};
      for (let i = 0; i < branch.length; i += 1) {
        data = {
          ...data,
          ...branch[i].data
        };
        props[`data_${i}`] = data;
      }
      props.page = {
        error: error2,
        params: event.params,
        route: event.route,
        status,
        url: event.url,
        data,
        form: form_value
      };
      const print_error = (property, replacement) => {
        Object.defineProperty(props.page, property, {
          get: () => {
            throw new Error(`$page.${property} has been replaced by $page.url.${replacement}`);
          }
        });
      };
      print_error("origin", "origin");
      print_error("path", "pathname");
      print_error("query", "searchParams");
      rendered = options.root.render(props);
      for (const { node } of branch) {
        if (node.imports) {
          node.imports.forEach((url) => modulepreloads.add(url));
        }
        if (node.stylesheets) {
          node.stylesheets.forEach((url) => stylesheets.add(url));
        }
        if (node.fonts) {
          node.fonts.forEach((url) => fonts.add(url));
        }
        if (node.inline_styles) {
          Object.entries(await node.inline_styles()).forEach(([k, v]) => inline_styles.set(k, v));
        }
      }
    } else {
      rendered = {
        head: "",
        html: "",
        css: {
          code: "",
          map: null
        }
      };
    }
    let head = "";
    let body = rendered.html;
    const csp = new Csp(options.csp, {
      dev: options.dev,
      prerender: !!state.prerendering
    });
    const target = hash(body);
    let assets2;
    if (options.paths.assets) {
      assets2 = options.paths.assets;
    } else if ((_a = state.prerendering) == null ? void 0 : _a.fallback) {
      assets2 = options.paths.base;
    } else {
      const segments = event.url.pathname.slice(options.paths.base.length).split("/").slice(2);
      assets2 = segments.length > 0 ? segments.map(() => "..").join("/") : ".";
    }
    const prefixed = (path) => path.startsWith("/") ? path : `${assets2}/${path}`;
    const serialized = {
      data: "",
      form: "null"
    };
    try {
      serialized.data = `[${branch.map(({ server_data }) => {
        if ((server_data == null ? void 0 : server_data.type) === "data") {
          const data = devalue.uneval(server_data.data);
          const uses = [];
          if (server_data.uses.dependencies.size > 0) {
            uses.push(`dependencies:${s(Array.from(server_data.uses.dependencies))}`);
          }
          if (server_data.uses.params.size > 0) {
            uses.push(`params:${s(Array.from(server_data.uses.params))}`);
          }
          if (server_data.uses.parent)
            uses.push(`parent:1`);
          if (server_data.uses.route)
            uses.push(`route:1`);
          if (server_data.uses.url)
            uses.push(`url:1`);
          return `{type:"data",data:${data},uses:{${uses.join(",")}}${server_data.slash ? `,slash:${s(server_data.slash)}` : ""}}`;
        }
        return s(server_data);
      }).join(",")}]`;
    } catch (e) {
      const error3 = e;
      throw new Error(clarify_devalue_error(event, error3));
    }
    if (form_value) {
      serialized.form = uneval_action_response(form_value, event.route.id);
    }
    if (inline_styles.size > 0) {
      const content = Array.from(inline_styles.values()).join("\n");
      const attributes = [];
      if (options.dev)
        attributes.push(" data-sveltekit");
      if (csp.style_needs_nonce)
        attributes.push(` nonce="${csp.nonce}"`);
      csp.add_style(content);
      head += `
	<style${attributes.join("")}>${content}</style>`;
    }
    for (const dep of stylesheets) {
      const path = prefixed(dep);
      if (resolve_opts.preload({
        type: "css",
        path
      })) {
        const attributes = [];
        if (csp.style_needs_nonce) {
          attributes.push(`nonce="${csp.nonce}"`);
        }
        if (inline_styles.has(dep)) {
          attributes.push("disabled", 'media="(max-width: 0)"');
        } else {
          const preload_atts = [
            'rel="preload"',
            'as="style"'
          ].concat(attributes);
          link_header_preloads.add(`<${encodeURI(path)}>; ${preload_atts.join(";")}; nopush`);
        }
        attributes.unshift('rel="stylesheet"');
        head += `
		<link href="${path}" ${attributes.join(" ")}>`;
      }
    }
    for (const dep of fonts) {
      const path = prefixed(dep);
      if (resolve_opts.preload({
        type: "font",
        path
      })) {
        const ext = dep.slice(dep.lastIndexOf(".") + 1);
        const attributes = [
          'rel="preload"',
          'as="font"',
          `type="font/${ext}"`,
          `href="${path}"`,
          "crossorigin"
        ];
        head += `
		<link ${attributes.join(" ")}>`;
      }
    }
    if (page_config.csr) {
      const init_app = `
			import { start } from ${s(prefixed(entry.file))};

			start({
				env: ${s(options.public_env)},
				hydrate: ${page_config.ssr ? `{
					status: ${status},
					error: ${devalue.uneval(error2)},
					node_ids: [${branch.map(({ node }) => node.index).join(", ")}],
					params: ${devalue.uneval(event.params)},
					route: ${s(event.route)},
					data: ${serialized.data},
					form: ${serialized.form}
				}` : "null"},
				paths: ${s(options.paths)},
				target: document.querySelector('[data-sveltekit-hydrate="${target}"]').parentNode,
				version: ${s(options.version)}
			});
		`;
      for (const dep of modulepreloads) {
        const path = prefixed(dep);
        if (resolve_opts.preload({
          type: "js",
          path
        })) {
          link_header_preloads.add(`<${encodeURI(path)}>; rel="modulepreload"; nopush`);
          if (state.prerendering) {
            head += `
		<link rel="modulepreload" href="${path}">`;
          }
        }
      }
      const attributes = [
        'type="module"',
        `data-sveltekit-hydrate="${target}"`
      ];
      csp.add_script(init_app);
      if (csp.script_needs_nonce) {
        attributes.push(`nonce="${csp.nonce}"`);
      }
      body += `
		<script ${attributes.join(" ")}>${init_app}<\/script>`;
    }
    if (page_config.ssr && page_config.csr) {
      body += `
	${fetched.map((item) => serialize_data(item, resolve_opts.filterSerializedResponseHeaders, !!state.prerendering)).join("\n	")}`;
    }
    if (options.service_worker) {
      const opts = options.dev ? `, { type: 'module' }` : "";
      const init_service_worker = `
			if ('serviceWorker' in navigator) {
				addEventListener('load', function () {
					navigator.serviceWorker.register('${prefixed("service-worker.js")}'${opts});
				});
			}
		`;
      csp.add_script(init_service_worker);
      head += `
		<script${csp.script_needs_nonce ? ` nonce="${csp.nonce}"` : ""}>${init_service_worker}<\/script>`;
    }
    if (state.prerendering) {
      const http_equiv = [];
      const csp_headers = csp.csp_provider.get_meta();
      if (csp_headers) {
        http_equiv.push(csp_headers);
      }
      if (state.prerendering.cache) {
        http_equiv.push(`<meta http-equiv="cache-control" content="${state.prerendering.cache}">`);
      }
      if (http_equiv.length > 0) {
        head = http_equiv.join("\n") + head;
      }
    }
    head += rendered.head;
    const html = await resolve_opts.transformPageChunk({
      html: options.app_template({
        head,
        body,
        assets: assets2,
        nonce: csp.nonce
      }),
      done: true
    }) || "";
    const headers = new Headers({
      "x-sveltekit-page": "true",
      "content-type": "text/html",
      etag: `"${hash(html)}"`
    });
    if (!state.prerendering) {
      const csp_header = csp.csp_provider.get_header();
      if (csp_header) {
        headers.set("content-security-policy", csp_header);
      }
      const report_only_header = csp.report_only_provider.get_header();
      if (report_only_header) {
        headers.set("content-security-policy-report-only", report_only_header);
      }
      if (link_header_preloads.size) {
        headers.set("link", Array.from(link_header_preloads).join(", "));
      }
    }
    return new Response(html, {
      status,
      headers
    });
  }
  async function respond_with_error({ event, options, state, status, error: error2, resolve_opts }) {
    const fetched = [];
    try {
      const branch = [];
      const default_layout = await options.manifest._.nodes[0]();
      const ssr = get_option([
        default_layout
      ], "ssr") ?? true;
      const csr = get_option([
        default_layout
      ], "csr") ?? true;
      if (ssr) {
        state.initiator = GENERIC_ERROR;
        const server_data_promise = load_server_data({
          event,
          state,
          node: default_layout,
          parent: async () => ({})
        });
        const server_data = await server_data_promise;
        const data = await load_data({
          event,
          fetched,
          node: default_layout,
          parent: async () => ({}),
          resolve_opts,
          server_data_promise,
          state,
          csr
        });
        branch.push({
          node: default_layout,
          server_data,
          data
        }, {
          node: await options.manifest._.nodes[1](),
          data: null,
          server_data: null
        });
      }
      return await render_response({
        options,
        state,
        page_config: {
          ssr,
          csr: get_option([
            default_layout
          ], "csr") ?? true
        },
        status,
        error: handle_error_and_jsonify(event, options, error2),
        branch,
        fetched,
        event,
        resolve_opts
      });
    } catch (error3) {
      if (error3 instanceof Redirect) {
        return redirect_response(error3.status, error3.location);
      }
      return static_error_page(options, error3 instanceof HttpError ? error3.status : 500, handle_error_and_jsonify(event, options, error3).message);
    }
  }
  async function render_page(event, route, page, options, state, resolve_opts) {
    if (state.initiator === route) {
      return new Response(`Not found: ${event.url.pathname}`, {
        status: 404
      });
    }
    state.initiator = route;
    if (is_action_json_request(event)) {
      const node = await options.manifest._.nodes[page.leaf]();
      if (node.server) {
        return handle_action_json_request(event, options, node.server);
      }
    }
    try {
      const nodes = await Promise.all([
        ...page.layouts.map((n) => n == void 0 ? n : options.manifest._.nodes[n]()),
        options.manifest._.nodes[page.leaf]()
      ]);
      const leaf_node = nodes.at(-1);
      let status = 200;
      let action_result = void 0;
      if (is_action_request(event, leaf_node)) {
        action_result = await handle_action_request(event, leaf_node.server);
        if ((action_result == null ? void 0 : action_result.type) === "redirect") {
          return redirect_response(303, action_result.location);
        }
        if ((action_result == null ? void 0 : action_result.type) === "error") {
          const error2 = action_result.error;
          status = error2 instanceof HttpError ? error2.status : 500;
        }
        if ((action_result == null ? void 0 : action_result.type) === "invalid") {
          status = action_result.status;
        }
      }
      const should_prerender_data = nodes.some((node) => node == null ? void 0 : node.server);
      const data_pathname = add_data_suffix(event.url.pathname);
      const should_prerender = get_option(nodes, "prerender") ?? false;
      if (should_prerender) {
        const mod = leaf_node.server;
        if (mod && mod.actions) {
          throw new Error("Cannot prerender pages with actions");
        }
      } else if (state.prerendering) {
        return new Response(void 0, {
          status: 204
        });
      }
      state.prerender_default = should_prerender;
      const fetched = [];
      if (get_option(nodes, "ssr") === false) {
        return await render_response({
          branch: [],
          fetched,
          page_config: {
            ssr: false,
            csr: get_option(nodes, "csr") ?? true
          },
          status,
          error: null,
          event,
          options,
          state,
          resolve_opts
        });
      }
      let branch = [];
      let load_error = null;
      const server_promises = nodes.map((node, i) => {
        if (load_error) {
          throw load_error;
        }
        return Promise.resolve().then(async () => {
          try {
            if (node === leaf_node && (action_result == null ? void 0 : action_result.type) === "error") {
              throw action_result.error;
            }
            return await load_server_data({
              event,
              state,
              node,
              parent: async () => {
                const data = {};
                for (let j = 0; j < i; j += 1) {
                  const parent = await server_promises[j];
                  if (parent)
                    Object.assign(data, await parent.data);
                }
                return data;
              }
            });
          } catch (e) {
            load_error = e;
            throw load_error;
          }
        });
      });
      const csr = get_option(nodes, "csr") ?? true;
      const load_promises = nodes.map((node, i) => {
        if (load_error)
          throw load_error;
        return Promise.resolve().then(async () => {
          try {
            return await load_data({
              event,
              fetched,
              node,
              parent: async () => {
                const data = {};
                for (let j = 0; j < i; j += 1) {
                  Object.assign(data, await load_promises[j]);
                }
                return data;
              },
              resolve_opts,
              server_data_promise: server_promises[i],
              state,
              csr
            });
          } catch (e) {
            load_error = e;
            throw load_error;
          }
        });
      });
      for (const p of server_promises)
        p.catch(() => {
        });
      for (const p of load_promises)
        p.catch(() => {
        });
      for (let i = 0; i < nodes.length; i += 1) {
        const node = nodes[i];
        if (node) {
          try {
            const server_data = await server_promises[i];
            const data = await load_promises[i];
            branch.push({
              node,
              server_data,
              data
            });
          } catch (e) {
            const err = normalize_error(e);
            if (err instanceof Redirect) {
              if (state.prerendering && should_prerender_data) {
                const body = JSON.stringify({
                  type: "redirect",
                  location: err.location
                });
                state.prerendering.dependencies.set(data_pathname, {
                  response: new Response(body),
                  body
                });
              }
              return redirect_response(err.status, err.location);
            }
            const status2 = err instanceof HttpError ? err.status : 500;
            const error2 = handle_error_and_jsonify(event, options, err);
            while (i--) {
              if (page.errors[i]) {
                const index = page.errors[i];
                const node2 = await options.manifest._.nodes[index]();
                let j = i;
                while (!branch[j])
                  j -= 1;
                return await render_response({
                  event,
                  options,
                  state,
                  resolve_opts,
                  page_config: {
                    ssr: true,
                    csr: true
                  },
                  status: status2,
                  error: error2,
                  branch: compact(branch.slice(0, j + 1)).concat({
                    node: node2,
                    data: null,
                    server_data: null
                  }),
                  fetched
                });
              }
            }
            return static_error_page(options, status2, error2.message);
          }
        } else {
          branch.push(null);
        }
      }
      if (state.prerendering && should_prerender_data) {
        const body = `{"type":"data","nodes":[${branch.map((node) => serialize_data_node(node == null ? void 0 : node.server_data)).join(",")}]}`;
        state.prerendering.dependencies.set(data_pathname, {
          response: new Response(body),
          body
        });
      }
      return await render_response({
        event,
        options,
        state,
        resolve_opts,
        page_config: {
          csr: get_option(nodes, "csr") ?? true,
          ssr: true
        },
        status,
        error: null,
        branch: compact(branch),
        action_result,
        fetched
      });
    } catch (error2) {
      return await respond_with_error({
        event,
        options,
        state,
        status: 500,
        error: error2,
        resolve_opts
      });
    }
  }
  function exec(match, params, matchers) {
    const result = {};
    const values = match.slice(1);
    let buffered = "";
    for (let i = 0; i < params.length; i += 1) {
      const param = params[i];
      let value = values[i];
      if (param.chained && param.rest && buffered) {
        value = value ? buffered + "/" + value : buffered;
      }
      buffered = "";
      if (value === void 0) {
        if (param.rest)
          result[param.name] = "";
      } else {
        if (param.matcher && !matchers[param.matcher](value)) {
          if (param.optional && param.chained) {
            let j = values.indexOf(void 0, i);
            if (j === -1) {
              const next = params[i + 1];
              if ((next == null ? void 0 : next.rest) && next.chained) {
                buffered = value;
              } else {
                return;
              }
            }
            while (j >= i) {
              values[j] = values[j - 1];
              j -= 1;
            }
            continue;
          }
          return;
        }
        result[param.name] = value;
      }
    }
    if (buffered)
      return;
    return result;
  }
  function once(fn) {
    let done = false;
    let result;
    return () => {
      if (done)
        return result;
      done = true;
      return result = fn();
    };
  }
  const INVALIDATED_HEADER = "x-sveltekit-invalidated";
  async function render_data(event, route, options, state, trailing_slash) {
    var _a;
    if (!route.page) {
      return new Response(void 0, {
        status: 404
      });
    }
    try {
      const node_ids = [
        ...route.page.layouts,
        route.page.leaf
      ];
      const invalidated = ((_a = event.request.headers.get(INVALIDATED_HEADER)) == null ? void 0 : _a.split(",").map(Boolean)) ?? node_ids.map(() => true);
      let aborted = false;
      const url = new URL(event.url);
      url.pathname = normalize_path(strip_data_suffix(url.pathname), trailing_slash);
      const new_event = {
        ...event,
        url
      };
      const functions = node_ids.map((n, i) => {
        return once(async () => {
          try {
            if (aborted) {
              return {
                type: "skip"
              };
            }
            const node = n == void 0 ? n : await options.manifest._.nodes[n]();
            return load_server_data({
              event: new_event,
              state,
              node,
              parent: async () => {
                const data = {};
                for (let j = 0; j < i; j += 1) {
                  const parent = await functions[j]();
                  if (parent) {
                    Object.assign(data, parent.data);
                  }
                }
                return data;
              }
            });
          } catch (e) {
            aborted = true;
            throw e;
          }
        });
      });
      const promises = functions.map(async (fn, i) => {
        if (!invalidated[i]) {
          return {
            type: "skip"
          };
        }
        return fn();
      });
      let length = promises.length;
      const nodes = await Promise.all(promises.map((p, i) => p.catch((error2) => {
        if (error2 instanceof Redirect) {
          throw error2;
        }
        length = Math.min(length, i + 1);
        return {
          type: "error",
          error: handle_error_and_jsonify(event, options, error2),
          status: error2 instanceof HttpError ? error2.status : void 0
        };
      })));
      try {
        const stubs = nodes.slice(0, length).map(serialize_data_node);
        const json2 = `{"type":"data","nodes":[${stubs.join(",")}]}`;
        return json_response(json2);
      } catch (e) {
        const error2 = e;
        return json_response(JSON.stringify(clarify_devalue_error(event, error2)), 500);
      }
    } catch (e) {
      const error2 = normalize_error(e);
      if (error2 instanceof Redirect) {
        return json_response(JSON.stringify({
          type: "redirect",
          location: error2.location
        }));
      } else {
        return json_response(JSON.stringify(handle_error_and_jsonify(event, options, error2)));
      }
    }
  }
  function json_response(json2, status = 200) {
    return new Response(json2, {
      status,
      headers: {
        "content-type": "application/json",
        "cache-control": "private, no-store"
      }
    });
  }
  const cookie_paths = {};
  const encode = encodeURIComponent;
  const decode = decodeURIComponent;
  function get_cookies(request, url, dev, trailing_slash) {
    const header = request.headers.get("cookie") ?? "";
    const initial_cookies = parse(header, {
      decode
    });
    const normalized_url = normalize_path(has_data_suffix(url.pathname) ? strip_data_suffix(url.pathname) : url.pathname, trailing_slash);
    const default_path = normalized_url.split("/").slice(0, -1).join("/") || "/";
    if (dev) {
      for (const name of Object.keys(cookie_paths)) {
        cookie_paths[name] = new Set([
          ...cookie_paths[name]
        ].filter((path) => !path_matches(normalized_url, path) || name in initial_cookies));
      }
      for (const name in initial_cookies) {
        cookie_paths[name] = cookie_paths[name] ?? /* @__PURE__ */ new Set();
        if (![
          ...cookie_paths[name]
        ].some((path) => path_matches(normalized_url, path))) {
          cookie_paths[name].add(default_path);
        }
      }
    }
    const new_cookies = {};
    const defaults = {
      httpOnly: true,
      sameSite: "lax",
      secure: url.hostname === "localhost" && url.protocol === "http:" ? false : true
    };
    const cookies = {
      get(name, opts) {
        const c = new_cookies[name];
        if (c && domain_matches(url.hostname, c.options.domain) && path_matches(url.pathname, c.options.path)) {
          return c.value;
        }
        const decoder = (opts == null ? void 0 : opts.decode) || decode;
        const req_cookies = parse(header, {
          decode: decoder
        });
        const cookie = req_cookies[name];
        if (!dev || cookie) {
          return cookie;
        }
        const paths = /* @__PURE__ */ new Set([
          ...cookie_paths[name] ?? []
        ]);
        if (c) {
          paths.add(c.options.path ?? default_path);
        }
        if (paths.size > 0) {
          console.warn(`Cookie with name '${name}' was not found at path '${url.pathname}', but a cookie with that name exists at these paths: '${[
            ...paths
          ].join("', '")}'. Did you mean to set its 'path' to '/' instead?`);
        }
      },
      set(name, value, opts = {}) {
        let path = opts.path ?? default_path;
        new_cookies[name] = {
          name,
          value,
          options: {
            ...defaults,
            ...opts,
            path
          }
        };
        if (dev) {
          cookie_paths[name] = cookie_paths[name] ?? /* @__PURE__ */ new Set();
          if (!value) {
            if (!cookie_paths[name].has(path) && cookie_paths[name].size > 0) {
              const paths = `'${Array.from(cookie_paths[name]).join("', '")}'`;
              console.warn(`Trying to delete cookie '${name}' at path '${path}', but a cookie with that name only exists at these paths: ${paths}.`);
            }
            cookie_paths[name].delete(path);
          } else {
            cookie_paths[name].add(path);
          }
        }
      },
      delete(name, opts = {}) {
        cookies.set(name, "", {
          ...opts,
          maxAge: 0
        });
      },
      serialize(name, value, opts) {
        return serialize(name, value, {
          ...defaults,
          ...opts
        });
      }
    };
    function get_cookie_header(destination, header2) {
      const combined_cookies = {};
      for (const name in initial_cookies) {
        combined_cookies[name] = encode(initial_cookies[name]);
      }
      for (const key2 in new_cookies) {
        const cookie = new_cookies[key2];
        if (!domain_matches(destination.hostname, cookie.options.domain))
          continue;
        if (!path_matches(destination.pathname, cookie.options.path))
          continue;
        const encoder2 = cookie.options.encode || encode;
        combined_cookies[cookie.name] = encoder2(cookie.value);
      }
      if (header2) {
        const parsed = parse(header2, {
          decode
        });
        for (const name in parsed) {
          combined_cookies[name] = encode(parsed[name]);
        }
      }
      return Object.entries(combined_cookies).map(([name, value]) => `${name}=${value}`).join("; ");
    }
    return {
      cookies,
      new_cookies,
      get_cookie_header
    };
  }
  function domain_matches(hostname, constraint) {
    if (!constraint)
      return true;
    const normalized = constraint[0] === "." ? constraint.slice(1) : constraint;
    if (hostname === normalized)
      return true;
    return hostname.endsWith("." + normalized);
  }
  function path_matches(path, constraint) {
    if (!constraint)
      return true;
    const normalized = constraint.endsWith("/") ? constraint.slice(0, -1) : constraint;
    if (path === normalized)
      return true;
    return path.startsWith(normalized + "/");
  }
  function add_cookies_to_headers(headers, cookies) {
    for (const new_cookie of cookies) {
      const { name, value, options } = new_cookie;
      headers.append("set-cookie", serialize(name, value, options));
    }
  }
  function create_fetch({ event, options, state, get_cookie_header }) {
    return async (info, init2) => {
      const original_request = normalize_fetch_input(info, init2, event.url);
      const request_body = init2 == null ? void 0 : init2.body;
      let mode = (info instanceof Request ? info.mode : init2 == null ? void 0 : init2.mode) ?? "cors";
      let credentials = (info instanceof Request ? info.credentials : init2 == null ? void 0 : init2.credentials) ?? "same-origin";
      return await options.hooks.handleFetch({
        event,
        request: original_request,
        fetch: async (info2, init3) => {
          const request = normalize_fetch_input(info2, init3, event.url);
          const url = new URL(request.url);
          if (!request.headers.has("origin")) {
            request.headers.set("origin", event.url.origin);
          }
          if (info2 !== original_request) {
            mode = (info2 instanceof Request ? info2.mode : init3 == null ? void 0 : init3.mode) ?? "cors";
            credentials = (info2 instanceof Request ? info2.credentials : init3 == null ? void 0 : init3.credentials) ?? "same-origin";
          }
          if ((request.method === "GET" || request.method === "HEAD") && (mode === "no-cors" && url.origin !== event.url.origin || url.origin === event.url.origin)) {
            request.headers.delete("origin");
          }
          if (url.origin !== event.url.origin) {
            if (`.${url.hostname}`.endsWith(`.${event.url.hostname}`) && credentials !== "omit") {
              const cookie = get_cookie_header(url, request.headers.get("cookie"));
              if (cookie)
                request.headers.set("cookie", cookie);
            }
            let response2 = await fetch(request);
            if (mode === "no-cors") {
              response2 = new Response("", {
                status: response2.status,
                statusText: response2.statusText,
                headers: response2.headers
              });
            }
            return response2;
          }
          let response;
          const prefix = options.paths.assets || options.paths.base;
          const decoded = decodeURIComponent(url.pathname);
          const filename = (decoded.startsWith(prefix) ? decoded.slice(prefix.length) : decoded).slice(1);
          const filename_html = `${filename}/index.html`;
          const is_asset = options.manifest.assets.has(filename);
          const is_asset_html = options.manifest.assets.has(filename_html);
          if (is_asset || is_asset_html) {
            const file = is_asset ? filename : filename_html;
            if (options.read) {
              const type = is_asset ? options.manifest.mimeTypes[filename.slice(filename.lastIndexOf("."))] : "text/html";
              return new Response(options.read(file), {
                headers: type ? {
                  "content-type": type
                } : {}
              });
            }
            return await fetch(request);
          }
          if (credentials !== "omit") {
            const cookie = get_cookie_header(url, request.headers.get("cookie"));
            if (cookie) {
              request.headers.set("cookie", cookie);
            }
            const authorization = event.request.headers.get("authorization");
            if (authorization && !request.headers.has("authorization")) {
              request.headers.set("authorization", authorization);
            }
          }
          if (request_body && typeof request_body !== "string" && !ArrayBuffer.isView(request_body)) {
            throw new Error("Request body must be a string or TypedArray");
          }
          if (!request.headers.has("accept")) {
            request.headers.set("accept", "*/*");
          }
          if (!request.headers.has("accept-language")) {
            request.headers.set("accept-language", event.request.headers.get("accept-language"));
          }
          response = await respond(request, options, state);
          const set_cookie = response.headers.get("set-cookie");
          if (set_cookie) {
            for (const str of set_cookie_parser.splitCookiesString(set_cookie)) {
              const { name, value, ...options2 } = set_cookie_parser.parseString(str);
              event.cookies.set(name, value, options2);
            }
          }
          return response;
        }
      });
    };
  }
  function normalize_fetch_input(info, init2, url) {
    if (info instanceof Request) {
      return info;
    }
    return new Request(typeof info === "string" ? new URL(info, url) : info, init2);
  }
  const default_transform = ({ html }) => html;
  const default_filter = () => false;
  const default_preload = ({ type }) => type === "js" || type === "css";
  async function respond(request, options, state) {
    var _a, _b, _c;
    let url = new URL(request.url);
    if (options.csrf.check_origin) {
      const forbidden = request.method === "POST" && request.headers.get("origin") !== url.origin && is_form_content_type(request);
      if (forbidden) {
        return new Response(`Cross-site ${request.method} form submissions are forbidden`, {
          status: 403
        });
      }
    }
    let decoded;
    try {
      decoded = decode_pathname(url.pathname);
    } catch {
      return new Response("Malformed URI", {
        status: 400
      });
    }
    let route = null;
    let params = {};
    if (options.paths.base && !((_a = state.prerendering) == null ? void 0 : _a.fallback)) {
      if (!decoded.startsWith(options.paths.base)) {
        return new Response("Not found", {
          status: 404
        });
      }
      decoded = decoded.slice(options.paths.base.length) || "/";
    }
    const is_data_request = has_data_suffix(decoded);
    if (is_data_request)
      decoded = strip_data_suffix(decoded) || "/";
    if (!((_b = state.prerendering) == null ? void 0 : _b.fallback)) {
      const matchers = await options.manifest._.matchers();
      for (const candidate of options.manifest._.routes) {
        const match = candidate.pattern.exec(decoded);
        if (!match)
          continue;
        const matched = exec(match, candidate.params, matchers);
        if (matched) {
          route = candidate;
          params = decode_params(matched);
          break;
        }
      }
    }
    let trailing_slash = void 0;
    const headers = {};
    const event = {
      cookies: null,
      fetch: null,
      getClientAddress: state.getClientAddress || (() => {
        throw new Error(`${"@sveltejs/adapter-static"} does not specify getClientAddress. Please raise an issue`);
      }),
      locals: {},
      params,
      platform: state.platform,
      request,
      route: {
        id: (route == null ? void 0 : route.id) ?? null
      },
      setHeaders: (new_headers) => {
        for (const key2 in new_headers) {
          const lower = key2.toLowerCase();
          const value = new_headers[key2];
          if (lower === "set-cookie") {
            throw new Error(`Use \`event.cookies.set(name, value, options)\` instead of \`event.setHeaders\` to set cookies`);
          } else if (lower in headers) {
            throw new Error(`"${key2}" header is already set`);
          } else {
            headers[lower] = value;
            if (state.prerendering && lower === "cache-control") {
              state.prerendering.cache = value;
            }
          }
        }
      },
      url
    };
    const removed = (property, replacement, suffix = "") => ({
      get: () => {
        throw new Error(`event.${property} has been replaced by event.${replacement}` + suffix);
      }
    });
    const details = ". See https://github.com/sveltejs/kit/pull/3384 for details";
    const body_getter = {
      get: () => {
        throw new Error("To access the request body use the text/json/arrayBuffer/formData methods, e.g. `body = await request.json()`" + details);
      }
    };
    Object.defineProperties(event, {
      clientAddress: removed("clientAddress", "getClientAddress"),
      method: removed("method", "request.method", details),
      headers: removed("headers", "request.headers", details),
      origin: removed("origin", "url.origin"),
      path: removed("path", "url.pathname"),
      query: removed("query", "url.searchParams"),
      body: body_getter,
      rawBody: body_getter,
      routeId: removed("routeId", "route.id")
    });
    let resolve_opts = {
      transformPageChunk: default_transform,
      filterSerializedResponseHeaders: default_filter,
      preload: default_preload
    };
    try {
      if (route && !is_data_request) {
        if (route.page) {
          const nodes = await Promise.all([
            ...route.page.layouts.map((n) => n == void 0 ? n : options.manifest._.nodes[n]()),
            options.manifest._.nodes[route.page.leaf]()
          ]);
          trailing_slash = get_option(nodes, "trailingSlash");
        } else if (route.endpoint) {
          const node = await route.endpoint();
          trailing_slash = node.trailingSlash;
        }
        const normalized = normalize_path(url.pathname, trailing_slash ?? "never");
        if (normalized !== url.pathname && !((_c = state.prerendering) == null ? void 0 : _c.fallback)) {
          return new Response(void 0, {
            status: 301,
            headers: {
              "x-sveltekit-normalize": "1",
              location: (normalized.startsWith("//") ? url.origin + normalized : normalized) + (url.search === "?" ? "" : url.search)
            }
          });
        }
      }
      const { cookies, new_cookies, get_cookie_header } = get_cookies(request, url, options.dev, trailing_slash ?? "never");
      event.cookies = cookies;
      event.fetch = create_fetch({
        event,
        options,
        state,
        get_cookie_header
      });
      if (state.prerendering && !state.prerendering.fallback)
        disable_search(url);
      const response = await options.hooks.handle({
        event,
        resolve: (event2, opts) => resolve(event2, opts).then((response2) => {
          for (const key2 in headers) {
            const value = headers[key2];
            response2.headers.set(key2, value);
          }
          if (is_data_request) {
            const vary = response2.headers.get("Vary");
            if (vary !== "*") {
              response2.headers.append("Vary", INVALIDATED_HEADER);
            }
          }
          add_cookies_to_headers(response2.headers, Object.values(new_cookies));
          if (state.prerendering && event2.route.id !== null) {
            response2.headers.set("x-sveltekit-routeid", encodeURI(event2.route.id));
          }
          return response2;
        }),
        get request() {
          throw new Error("request in handle has been replaced with event" + details);
        }
      });
      if (response.status === 200 && response.headers.has("etag")) {
        let if_none_match_value = request.headers.get("if-none-match");
        if (if_none_match_value == null ? void 0 : if_none_match_value.startsWith('W/"')) {
          if_none_match_value = if_none_match_value.substring(2);
        }
        const etag = response.headers.get("etag");
        if (if_none_match_value === etag) {
          const headers2 = new Headers({
            etag
          });
          for (const key2 of [
            "cache-control",
            "content-location",
            "date",
            "expires",
            "vary",
            "set-cookie"
          ]) {
            const value = response.headers.get(key2);
            if (value)
              headers2.set(key2, value);
          }
          return new Response(void 0, {
            status: 304,
            headers: headers2
          });
        }
      }
      return response;
    } catch (error2) {
      if (error2 instanceof Redirect) {
        return redirect_response(error2.status, error2.location);
      }
      return handle_fatal_error(event, options, error2);
    }
    async function resolve(event2, opts) {
      var _a2;
      try {
        if (opts) {
          if ("transformPage" in opts) {
            throw new Error("transformPage has been replaced by transformPageChunk \u2014 see https://github.com/sveltejs/kit/pull/5657 for more information");
          }
          if ("ssr" in opts) {
            throw new Error("ssr has been removed, set it in the appropriate +layout.js instead. See the PR for more information: https://github.com/sveltejs/kit/pull/6197");
          }
          resolve_opts = {
            transformPageChunk: opts.transformPageChunk || default_transform,
            filterSerializedResponseHeaders: opts.filterSerializedResponseHeaders || default_filter,
            preload: opts.preload || default_preload
          };
        }
        if ((_a2 = state.prerendering) == null ? void 0 : _a2.fallback) {
          return await render_response({
            event: event2,
            options,
            state,
            page_config: {
              ssr: false,
              csr: true
            },
            status: 200,
            error: null,
            branch: [],
            fetched: [],
            resolve_opts
          });
        }
        if (route) {
          let response;
          if (is_data_request) {
            response = await render_data(event2, route, options, state, trailing_slash ?? "never");
          } else if (route.endpoint && (!route.page || is_endpoint_request(event2))) {
            response = await render_endpoint(event2, await route.endpoint(), state);
          } else if (route.page) {
            response = await render_page(event2, route, route.page, options, state, resolve_opts);
          } else {
            throw new Error("This should never happen");
          }
          return response;
        }
        if (state.initiator === GENERIC_ERROR) {
          return new Response("Internal Server Error", {
            status: 500
          });
        }
        if (!state.initiator) {
          return await respond_with_error({
            event: event2,
            options,
            state,
            status: 404,
            error: new Error(`Not found: ${event2.url.pathname}`),
            resolve_opts
          });
        }
        if (state.prerendering) {
          return new Response("not found", {
            status: 404
          });
        }
        return await fetch(request);
      } catch (error2) {
        return handle_fatal_error(event2, options, error2);
      } finally {
        event2.cookies.set = () => {
          throw new Error("Cannot use `cookies.set(...)` after the response has been generated");
        };
        event2.setHeaders = () => {
          throw new Error("Cannot use `setHeaders(...)` after the response has been generated");
        };
      }
    }
  }
  let base = "";
  let assets = "";
  function set_paths(paths) {
    base = paths.base;
    assets = paths.assets || base;
  }
  const app_template = ({ head, body, assets: assets2, nonce }) => '<!DOCTYPE html>\n<html lang="en">\n	<head>\n		<meta charset="utf-8" />\n		<link rel="icon" href="' + assets2 + '/favicon.png" />\n		<link href="' + assets2 + '/material-icons/index.css" rel="stylesheet">\n		<link rel="stylesheet" href="https://fonts.googleapis.com/css?family=Roboto:300,400,500,600,700" />\n		<link rel="stylesheet" href="https://fonts.googleapis.com/css?family=Roboto+Mono" />\n		<link href="' + assets2 + '/beercss/beer.min.css" rel="stylesheet" />\n		<link href="' + assets2 + '/theme.css" rel="stylesheet" />\n		<script src="' + assets2 + '/beercss/beer.min.js" type="text/javascript"><\/script>\n		<meta name="viewport" content="width=device-width" />\n		' + head + '\n	</head>\n	<body class="light">\n		<div style="display: contents" class="mdc-typography--font-family">' + body + "</div>\n	</body>\n</html>\n";
  const error_template = ({ status, message }) => '<!DOCTYPE html>\n<html lang="en">\n	<head>\n		<meta charset="utf-8" />\n		<title>' + message + `</title>

		<style>
			body {
				font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen,
					Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
				display: flex;
				align-items: center;
				justify-content: center;
				height: 100vh;
			}

			.error {
				display: flex;
				align-items: center;
				max-width: 32rem;
				margin: 0 1rem;
			}

			.status {
				font-weight: 200;
				font-size: 3rem;
				line-height: 1;
				position: relative;
				top: -0.05rem;
			}

			.message {
				border-left: 1px solid #ccc;
				padding: 0 0 0 1rem;
				margin: 0 0 0 1rem;
				min-height: 2.5rem;
				display: flex;
				align-items: center;
			}

			.message h1 {
				font-weight: 400;
				font-size: 1em;
				margin: 0;
			}
		</style>
	</head>
	<body>
		<div class="error">
			<span class="status">` + status + '</span>\n			<div class="message">\n				<h1>' + message + "</h1>\n			</div>\n		</div>\n	</body>\n</html>\n";
  let read = null;
  set_paths({
    "base": "",
    "assets": ""
  });
  let default_protocol = "https";
  override = function override2(settings) {
    default_protocol = settings.protocol || default_protocol;
    set_paths(settings.paths);
    set_building(settings.building);
    read = settings.read;
  };
  Server = class Server {
    constructor(manifest) {
      this.options = {
        csp: {
          "mode": "auto",
          "directives": {
            "upgrade-insecure-requests": false,
            "block-all-mixed-content": false
          },
          "reportOnly": {
            "upgrade-insecure-requests": false,
            "block-all-mixed-content": false
          }
        },
        csrf: {
          check_origin: true
        },
        dev: false,
        handle_error: (error2, event) => {
          return this.options.hooks.handleError({
            error: error2,
            event,
            get request() {
              throw new Error("request in handleError has been replaced with event. See https://github.com/sveltejs/kit/pull/3384 for details");
            }
          }) ?? {
            message: event.route.id != null ? "Internal Error" : "Not Found"
          };
        },
        hooks: null,
        manifest,
        paths: {
          base,
          assets
        },
        public_env: {},
        read,
        root: Root,
        service_worker: false,
        app_template,
        app_template_contains_nonce: false,
        error_template,
        version: "1669941127091"
      };
    }
    async init({ env }) {
      const entries = Object.entries(env);
      Object.fromEntries(entries.filter(([k]) => !k.startsWith("PUBLIC_")));
      const pub = Object.fromEntries(entries.filter(([k]) => k.startsWith("PUBLIC_")));
      this.options.public_env = pub;
      if (!this.options.hooks) {
        const module = await import("./chunks/hooks.js");
        if (module.externalFetch) {
          throw new Error("externalFetch has been removed \u2014 use handleFetch instead. See https://github.com/sveltejs/kit/pull/6565 for details");
        }
        this.options.hooks = {
          handle: module.handle || (({ event, resolve }) => resolve(event)),
          handleError: module.handleError || (({ error: error2 }) => console.error(error2.stack)),
          handleFetch: module.handleFetch || (({ request, fetch: fetch2 }) => fetch2(request))
        };
      }
    }
    async respond(request, options = {}) {
      if (!(request instanceof Request)) {
        throw new Error("The first argument to server.respond must be a Request object. See https://github.com/sveltejs/kit/pull/3384 for details");
      }
      return respond(request, this.options, options);
    }
  };
})();
export {
  Server,
  __tla,
  override
};
