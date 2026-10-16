import { NpcapScreen } from "./components/NpcapScreen";
import { Shell } from "./components/Shell";
import { TitleBar } from "./components/TitleBar";
import { SettingsProvider } from "./lib/SettingsContext";
import { useNpcapStatus } from "./lib/useNpcapStatus";

function Gate() {
  const npcap = useNpcapStatus();
  const { result } = npcap;

  // Npcap ready (Windows) or not applicable (dev on macOS, replay): the app.
  if (result?.kind === "status") {
    const { status } = result;
    if (status.state === "found" || status.state === "notApplicable") {
      return <Shell npcap={status} />;
    }
  }
  return <NpcapScreen {...npcap} />;
}

export default function App() {
  return (
    <SettingsProvider>
      {/* The window frame: efsp's title bar above one rounded panel. */}
      <div
        // PingGraph watches this element to follow the window size.
        data-app-frame
        className="grid h-screen grid-rows-[2.25rem_1fr] bg-chrome"
      >
        <TitleBar />
        <div className="min-h-0 px-2 pb-2">
          <Gate />
        </div>
      </div>
    </SettingsProvider>
  );
}
