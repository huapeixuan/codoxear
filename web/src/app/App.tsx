import { AppProviders } from "./providers";
import { AppShell } from "./AppShell";

export default function App() {
  return (
    <AppProviders>
      <AppShell />
    </AppProviders>
  );
}
