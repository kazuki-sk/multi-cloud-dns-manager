import { useEffect, useState } from "react";

export type ThemePreference = "system" | "light" | "dark";

function getInitialPreference(): ThemePreference {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark" || stored === "system") return stored;
  return "system";
}

export function useTheme() {
  const [preference, setPreference] = useState<ThemePreference>(getInitialPreference);
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches
  );

  const theme: "light" | "dark" =
    preference === "system" ? (systemDark ? "dark" : "light") : preference;

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    if (preference === "system") {
      localStorage.removeItem("theme");
    } else {
      localStorage.setItem("theme", preference);
    }
  }, [theme, preference]);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  const toggleTheme = () => setPreference(theme === "dark" ? "light" : "dark");

  return { theme, preference, setThemePreference: setPreference, toggleTheme };
}
