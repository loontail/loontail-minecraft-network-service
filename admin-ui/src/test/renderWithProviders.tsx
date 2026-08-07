import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement, ReactNode } from "react";
import { MemoryRouter } from "react-router";

// `delay: null` drops user-event's inter-event yield, which the page tests pay for
// on every keystroke of a multi-character `type`.
export const setupUser = () => userEvent.setup({ delay: null });

export function makeTestQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
    },
  });
}

// Pass `client` to seed the cache before mount (the state a page reached via an
// in-app navigation starts from) or to drive refetches from a test.
export function renderWithProviders(
  ui: ReactElement,
  {
    route = "/",
    client = makeTestQueryClient(),
  }: { route?: string; client?: QueryClient } = {},
) {
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[route]}>{children}</MemoryRouter>
    </QueryClientProvider>
  );
  return { ...render(ui, { wrapper: Wrapper }), client };
}
