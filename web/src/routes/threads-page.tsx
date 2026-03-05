import { MessageSquareMore } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { useAppStore } from "@/context/app-store";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";

export function ThreadsPage() {
  const navigate = useNavigate();
  const { state } = useAppStore();

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto w-full max-w-5xl space-y-4 px-3 py-4 md:px-6 md:py-6">
        <Card>
          <CardHeader>
            <CardTitle className="display-heading text-2xl">
              Thread Explorer
            </CardTitle>
            <CardDescription>
              Additional feature route powered by TanStack Router. Use it to
              inspect and jump across sessions quickly.
            </CardDescription>
          </CardHeader>
        </Card>

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {state.threads.map((thread) => (
            <Card key={thread.id} className="bg-card/80">
              <CardHeader className="pb-2">
                <CardTitle className="line-clamp-1 text-base">
                  {thread.display_name}
                </CardTitle>
                <CardDescription>
                  {thread.message_count} messages
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Button
                  variant={
                    thread.id === state.currentSessionId ? "default" : "outline"
                  }
                  className="w-full"
                  onClick={() =>
                    navigate({ to: "/", search: { session: thread.id } })
                  }
                >
                  <MessageSquareMore className="h-4 w-4" />
                  Open Thread
                </Button>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    </ScrollArea>
  );
}
