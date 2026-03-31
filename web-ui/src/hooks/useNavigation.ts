import { useCallback } from "react";

function apiBase(): string {
  return "";
}

function fireAndForget(path: string, body?: object) {
  const base = apiBase();
  fetch(`${base}${path}`, {
    method: "POST",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  }).catch(() => {
    // fire-and-forget: state updates arrive via SSE
  });
}

export function useNavigation() {
  const goBack = useCallback(() => fireAndForget("/ui/chrome/back"), []);
  const goForward = useCallback(() => fireAndForget("/ui/chrome/forward"), []);
  const reload = useCallback(() => fireAndForget("/ui/chrome/reload"), []);
  const stop = useCallback(() => fireAndForget("/ui/chrome/stop"), []);
  const navigate = useCallback(
    (url: string) => fireAndForget("/ui/chrome/navigate", { url }),
    [],
  );

  return { goBack, goForward, reload, stop, navigate };
}
