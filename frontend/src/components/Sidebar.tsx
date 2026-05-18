import { NavLink } from "react-router-dom";
import { Globe, GitMerge, Server, Settings } from "lucide-react";

const NAV_ITEMS = [
  { to: "/zones", label: "Zones", icon: Globe },
  { to: "/changesets", label: "Changesets", icon: GitMerge },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/settings", label: "Settings", icon: Settings },
] as const;

export function Sidebar() {
  return (
    <nav className="flex h-full w-56 flex-col border-r border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      {/* Logo */}
      <div className="flex h-14 items-center gap-2 border-b border-gray-200 px-4 dark:border-gray-700">
        <Globe className="h-5 w-5 text-blue-500" />
        <span className="text-sm font-semibold text-gray-900 dark:text-white">
          DNS Manager
        </span>
      </div>

      {/* Navigation */}
      <ul className="flex-1 space-y-0.5 p-2">
        {NAV_ITEMS.map(({ to, label, icon: Icon }) => (
          <li key={to}>
            <NavLink
              to={to}
              className={({ isActive }) =>
                [
                  "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-blue-50 text-blue-600 dark:bg-blue-900/40 dark:text-blue-400"
                    : "text-gray-600 hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-100",
                ].join(" ")
              }
            >
              <Icon className="h-4 w-4 flex-shrink-0" />
              {label}
            </NavLink>
          </li>
        ))}
      </ul>
    </nav>
  );
}
