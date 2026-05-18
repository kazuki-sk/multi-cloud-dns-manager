import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { ThemeToggle } from "./ThemeToggle";

export function Layout() {
  return (
    <div className="flex h-screen bg-gray-50 dark:bg-gray-950">
      {/* Left sidebar */}
      <Sidebar />

      {/* Main area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Top bar */}
        <header className="flex h-14 items-center justify-end border-b border-gray-200 bg-white px-4 dark:border-gray-700 dark:bg-gray-900">
          <ThemeToggle />
        </header>

        {/* Page content */}
        <main className="flex-1 overflow-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
