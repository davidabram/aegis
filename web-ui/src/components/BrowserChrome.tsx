import { colors } from "../tokens";
import { useChromeState } from "../hooks/useChromeState";
import { useNavigation } from "../hooks/useNavigation";
import { Toolbar } from "./Toolbar";
import { PageTransition } from "./PageTransition";
import { RemoteViewport } from "./RemoteViewport";

export function BrowserChrome() {
  const state = useChromeState();
  const nav = useNavigation();

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        background: colors.windowBg,
        minWidth: 520,
        minHeight: 400,
      }}
    >
      <Toolbar
        title={state.title}
        url={state.url}
        canGoBack={state.canGoBack}
        canGoForward={state.canGoForward}
        isLoading={state.isLoading}
        onBack={nav.goBack}
        onForward={nav.goForward}
        onReload={nav.reload}
        onStop={nav.stop}
        onNavigate={nav.navigate}
      />

      <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
        <RemoteViewport />
        <PageTransition isLoading={state.isLoading} />
      </div>
    </div>
  );
}
