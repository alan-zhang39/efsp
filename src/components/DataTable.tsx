import { flexRender, type Row, type RowData, type Table as Model } from "@tanstack/react-table";
import { cn } from "cn";
import type { ComponentProps } from "react";
import { Button } from "@/components/ui/button";
import type { PickerSwitches } from "./ServersView";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

declare module "@tanstack/react-table" {
  // Type parameters must match TanStack's declaration to merge.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData extends RowData, TValue> {
    /** Right-aligned (numbers). */
    numeric?: boolean;
    /** This column's cell names its row: a <th scope="row">. */
    rowHeader?: boolean;
    /** Extra classes for the column's header and cells. */
    className?: string | undefined;
  }

  // What a page's cells need that changes while the columns don't (columns
  // are defined once: TanStack renders each cell function as a component).
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface TableMeta<TData extends RowData> {
    /** History: opens a saved match. */
    openMatch?: (id: number) => void;
    /** Servers: the picker's switches. */
    serverToggle?: PickerSwitches | null;
  }
}

/**
 * A table driven by a TanStack Table model: columns, sorting (with
 * `aria-sort` and a clickable header) and rows all come from `table`.
 */
export function DataTable<R>({
  table,
  caption,
  rowProps,
}: {
  table: Model<R>;
  /** Read by screen readers only. */
  caption: string;
  rowProps?: (row: Row<R>) => ComponentProps<"tr">;
}) {
  return (
    <Table>
      <caption className="sr-only">{caption}</caption>
      <TableHeader>
        {table.getHeaderGroups().map((group) => (
          <TableRow key={group.id}>
            {group.headers.map((header) => {
              const meta = header.column.columnDef.meta;
              const sorted = header.column.getIsSorted();
              const label = flexRender(header.column.columnDef.header, header.getContext());
              return (
                <TableHead
                  key={header.id}
                  scope="col"
                  aria-sort={
                    sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : undefined
                  }
                  className={cn(meta?.numeric === true && "text-right", meta?.className)}
                >
                  {header.column.getCanSort() ? (
                    <button
                      type="button"
                      className={cn(
                        "inline-flex cursor-pointer items-center gap-1 hover:text-foreground focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
                        sorted !== false && "text-foreground",
                        meta?.numeric === true && "flex-row-reverse",
                      )}
                      onClick={header.column.getToggleSortingHandler()}
                    >
                      {label}
                      <span className="inline-block min-w-[0.75em]" aria-hidden="true">
                        {sorted === "asc" ? "↑" : sorted === "desc" ? "↓" : ""}
                      </span>
                    </button>
                  ) : (
                    label
                  )}
                </TableHead>
              );
            })}
          </TableRow>
        ))}
      </TableHeader>
      <TableBody>
        {table.getRowModel().rows.map((row) => (
          <TableRow key={row.id} {...rowProps?.(row)}>
            {row.getVisibleCells().map((cell) => {
              const meta = cell.column.columnDef.meta;
              const content = flexRender(cell.column.columnDef.cell, cell.getContext());
              const className = cn(meta?.numeric === true && "text-right", meta?.className);
              return meta?.rowHeader === true ? (
                <th
                  key={cell.id}
                  scope="row"
                  className={cn(
                    "px-3 py-2.5 text-left align-middle font-normal whitespace-nowrap",
                    className,
                  )}
                >
                  {content}
                </th>
              ) : (
                <TableCell key={cell.id} className={className}>
                  {content}
                </TableCell>
              );
            })}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

/** "21–40 of 134" with previous / next. */
export function Pager({
  offset,
  pageSize,
  total,
  onOffset,
}: {
  offset: number;
  pageSize: number;
  total: number;
  onOffset: (offset: number) => void;
}) {
  if (total === 0) return null;
  const last = Math.min(offset + pageSize, total);
  return (
    <nav className="flex items-center justify-end gap-2" aria-label="Pages">
      <span className="mr-2 text-sm text-muted-foreground tabular-nums">
        {`${String(offset + 1)}–${String(last)} of ${String(total)}`}
      </span>
      <Button
        variant="quiet"
        disabled={offset === 0}
        onClick={() => {
          onOffset(Math.max(0, offset - pageSize));
        }}
      >
        Previous
      </Button>
      <Button
        variant="quiet"
        disabled={last >= total}
        onClick={() => {
          onOffset(offset + pageSize);
        }}
      >
        Next
      </Button>
    </nav>
  );
}
