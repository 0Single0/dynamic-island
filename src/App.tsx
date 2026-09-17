import { Route, Routes } from "react-router-dom";
import { AppToaster } from "@/components/common/AppToaster";
import { routes } from "@/routes";

export default function App() {
  return (
    <div className="h-screen w-screen overflow-hidden bg-transparent text-zinc-100 antialiased">
      <Routes>
        {routes.map((route) => (
          <Route
            key={route.path}
            path={route.path}
            element={<route.Component />}
          />
        ))}
      </Routes>
      <AppToaster />
    </div>
  );
}
