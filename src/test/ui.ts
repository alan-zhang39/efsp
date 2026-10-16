import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/** Picks `option` from the dropdown named `name` (a Base UI Select). */
export async function choose(name: string, option: string): Promise<void> {
  await userEvent.click(screen.getByRole("combobox", { name }));
  await userEvent.click(await screen.findByRole("option", { name: option }));
}
