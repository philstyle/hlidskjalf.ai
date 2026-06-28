import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

interface MarkdownRendererProps {
  content: string;
}

export default function MarkdownRenderer({ content }: MarkdownRendererProps) {
  return (
    <div
      className="prose prose-sm max-w-none
        prose-headings:text-nx-text prose-headings:font-heading
        prose-p:text-nx-text prose-p:font-body
        prose-a:text-nx-accent prose-a:no-underline hover:prose-a:underline
        prose-strong:text-nx-text
        prose-code:text-nx-accent prose-code:bg-nx-bg prose-code:px-1 prose-code:py-0.5 prose-code:rounded prose-code:text-xs prose-code:before:content-none prose-code:after:content-none
        prose-pre:bg-nx-bg prose-pre:border prose-pre:border-nx-border-light prose-pre:rounded-lg
        prose-blockquote:border-nx-accent/30 prose-blockquote:text-nx-text-secondary
        prose-hr:border-nx-border
        prose-th:text-nx-text prose-td:text-nx-text-secondary
        prose-li:text-nx-text prose-li:marker:text-nx-muted"
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
