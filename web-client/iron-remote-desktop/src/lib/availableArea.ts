export interface Corner {
    x: number;
    y: number;
}

/** Bottom-right corner of the window's viewport. */
export function windowCorner(win: Window = window): Corner {
    const docElem = win.document.documentElement;
    const body = win.document.getElementsByTagName('body')[0];
    return {
        x: win.innerWidth ?? docElem.clientWidth ?? body.clientWidth,
        y: win.innerHeight ?? docElem.clientHeight ?? body.clientHeight,
    };
}

/**
 * Bottom-right corner, in viewport coordinates, of the area the component may draw in.
 *
 * The fit and real scalings measure from the component's wrapper to this corner. When the component is
 * embedded in something smaller than the window (a split pane, a sidebar), that is the host element's box,
 * not the window: measuring to the window's corner would size the canvas past the host's edges. A host
 * without a size yet (not laid out) falls back to the window.
 */
export function availableAreaCorner(host: Element | null | undefined, win: Window = window): Corner {
    if (host) {
        const box = host.getBoundingClientRect();
        if (box.width > 0 && box.height > 0) {
            return { x: box.right, y: box.bottom };
        }
    }
    return windowCorner(win);
}
