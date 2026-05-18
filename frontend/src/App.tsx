import { Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./components/Layout";
import { ZonesPage } from "./pages/ZonesPage";
import { ZoneDetailPage } from "./pages/ZoneDetailPage";
import { ChangesetsPage } from "./pages/ChangesetsPage";
import { ProvidersPage } from "./pages/ProvidersPage";
import { SettingsPage } from "./pages/SettingsPage";
import { useTheme } from "./hooks/useTheme";

export default function App() {
  // Apply theme class on root element at app level.
  useTheme();

  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<Navigate to="/zones" replace />} />
        <Route path="zones" element={<ZonesPage />} />
        <Route path="zones/:id" element={<ZoneDetailPage />} />
        <Route path="changesets" element={<ChangesetsPage />} />
        <Route path="providers" element={<ProvidersPage />} />
        <Route path="settings" element={<SettingsPage />} />
      </Route>
    </Routes>
  );
}
