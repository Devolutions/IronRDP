/* tslint:disable */
/* eslint-disable */
/**
* @returns {DesktopSize}
*/
export function get_desktop_size(): DesktopSize;
/**
* @returns {Rect}
*/
export function dump_sample(): Rect;
/**
* @returns {Rect}
*/
export function next_rect(): Rect;
/**
* @param {string} username
* @param {string} password
* @param {string} address
*/
export function connect(username: string, password: string, address: string): void;
/**
*/
export function greet(): void;
/**
*/
export function init(): void;
/**
*/
export class DesktopSize {
  free(): void;
/**
*/
  height: number;
/**
*/
  width: number;
}
/**
*/
export class Rect {
  free(): void;
/**
* @returns {Uint8Array}
*/
  clone_buffer(): Uint8Array;
/**
*/
  bottom: number;
/**
*/
  left: number;
/**
*/
  right: number;
/**
*/
  top: number;
}
