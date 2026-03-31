/// <reference types="vite/client" />

declare module "novnc-next" {
  export default class RFB extends EventTarget {
    constructor(target: Element, url: string);
    scaleViewport: boolean;
    resizeSession: boolean;
    background: string;
    focusOnClick: boolean;
    clipViewport: boolean;
    disconnect(): void;
  }
}
