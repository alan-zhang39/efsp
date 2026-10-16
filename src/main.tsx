import { CSPProvider } from "@base-ui/react/csp-provider";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { TooltipProvider } from "./components/ui/tooltip";
import "./index.css";

const root = document.getElementById("root");
if (root === null) throw new Error("#root element missing from index.html");

createRoot(root).render(
  <StrictMode>
    {/* The app's CSP forbids inline <style> and <script>. The one style
        rule Base UI would inject lives in index.css instead. Base UI's Tabs
        indicator and Slider render an inline <script>: avoid them, or
        allow it explicitly, before using either. */}
    <CSPProvider disableStyleElements>
      <TooltipProvider>
        <App />
      </TooltipProvider>
    </CSPProvider>
  </StrictMode>,
);
