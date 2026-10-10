import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HarnessProvider } from "@/components/HarnessMark";
import { router } from "@/app/router";
import { logToDaemon } from "@/lib/api";
import { connectEvents } from "@/lib/ws";
import { useComposer } from "@/stores/composer";
// Reads `?ws=` before anything else rewrites the address.
import "@/stores/workspace";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    // Data is pushed over the WebSocket or refreshed explicitly; never refetch just because the window got focus.
    queries: { refetchOnWindowFocus: false, retry: 1, staleTime: 30_000 },
  },
});
connectEvents(queryClient);
void useComposer.getState().restore();

window.addEventListener("error", (e) => logToDaemon(`uncaught: ${e.message} at ${e.filename}:${e.lineno}`, "error"));
window.addEventListener("unhandledrejection", (e) => logToDaemon(`unhandled rejection: ${String(e.reason)}`, "error"));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <HarnessProvider><RouterProvider router={router} /></HarnessProvider>
    </QueryClientProvider>
  </StrictMode>,
);

// The desktop shell passes its start time (`lp_t0`, epoch ms) to measure the time to the first frame.
const params = new URLSearchParams(location.search);
const t0 = Number(params.get("lp_t0"));
if (t0) {
  params.delete("lp_t0");
  const rest = params.toString();
  history.replaceState(null, "", location.pathname + (rest ? `?${rest}` : "") + location.hash);
  requestAnimationFrame(() =>
    requestAnimationFrame(() =>
      logToDaemon(`window ready: ${Date.now() - t0} ms after the shell started (page ${Math.round(performance.now())} ms)`),
    ),
  );
}
