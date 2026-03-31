import RFB from "novnc-next";
import { useEffect, useRef, useState } from "react";

interface BootstrapResponse {
  websocket_path: string;
  can_reconnect: boolean;
}

function websocketUrl(path: string) {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}${path}`;
}

export function RemoteViewport() {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const rfbRef = useRef<RFB | null>(null);
  const [status, setStatus] = useState("Connecting to browser viewport");
  const [statusState, setStatusState] = useState<"info" | "error">("info");

  useEffect(() => {
    let cancelled = false;

    async function connect() {
      try {
        const response = await fetch("/ui/bootstrap");
        if (!response.ok) {
          throw new Error(`bootstrap failed with ${response.status}`);
        }
        const bootstrap = (await response.json()) as BootstrapResponse;
        if (cancelled || !containerRef.current) {
          return;
        }

        const rfb = new RFB(containerRef.current, websocketUrl(bootstrap.websocket_path));
        rfb.scaleViewport = true;
        rfb.resizeSession = true;
        rfb.background = "#11151a";
        rfb.focusOnClick = true;
        rfb.clipViewport = false;
        rfbRef.current = rfb;

        rfb.addEventListener("connect", () => {
          setStatus("Connected");
          setStatusState("info");
        });
        rfb.addEventListener("disconnect", () => {
          setStatus(
            bootstrap.can_reconnect ? "Viewport disconnected, retrying soon" : "Viewport disconnected",
          );
          setStatusState(bootstrap.can_reconnect ? "info" : "error");
        });
        rfb.addEventListener("securityfailure", () => {
          setStatus("VNC security negotiation failed");
          setStatusState("error");
        });
        rfb.addEventListener("credentialsrequired", () => {
          setStatus("Unexpected VNC credential request");
          setStatusState("error");
        });
      } catch (error) {
        setStatus(
          error instanceof Error ? error.message : "Failed to initialize browser viewport",
        );
        setStatusState("error");
      }
    }

    void connect();

    return () => {
      cancelled = true;
      rfbRef.current?.disconnect();
      rfbRef.current = null;
    };
  }, []);

  return (
    <div className="remote-viewport">
      <div className="remote-viewport__frame">
        <div className="remote-viewport__canvas" ref={containerRef} />
        <div className="remote-viewport__status" data-state={statusState}>
          {status}
        </div>
      </div>
    </div>
  );
}
