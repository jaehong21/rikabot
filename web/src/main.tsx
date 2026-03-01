import React from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";

import { AppStoreProvider } from "@/context/app-store";
import { router } from "@/router";
import "@/styles.css";

const rootElement = document.getElementById("app");

if (!rootElement) {
  throw new Error("App root element not found");
}

createRoot(rootElement).render(
  <React.StrictMode>
    <AppStoreProvider>
      <RouterProvider router={router} />
    </AppStoreProvider>
  </React.StrictMode>,
);
