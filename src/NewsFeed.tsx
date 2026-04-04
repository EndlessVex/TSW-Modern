import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

interface NewsPost {
  title: string;
  author: string;
  created_utc: number;
  permalink: string;
  score: number;
  num_comments: number;
}

/** Format a UTC epoch timestamp into a relative time string (e.g. "2h ago"). */
function relativeTime(epochSec: number): string {
  const now = Date.now() / 1000;
  const diff = Math.max(0, now - epochSec);

  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return `${Math.floor(diff / 604800)}w ago`;
}

export default function NewsFeed() {
  const [posts, setPosts] = useState<NewsPost[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    invoke<NewsPost[]>("fetch_news")
      .then((result) => {
        if (!cancelled) {
          setPosts(result);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(String(err));
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (loading) {
    return (
      <div className="news-feed">
        <div className="news-loading">Loading community news…</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="news-feed">
        <div className="news-error">Unable to load news feed.</div>
      </div>
    );
  }

  if (posts.length === 0) {
    return (
      <div className="news-feed">
        <div className="news-empty">No recent posts.</div>
      </div>
    );
  }

  return (
    <div className="news-feed">
      {posts.map((post, i) => (
        <a
          key={i}
          className="news-card"
          href={`https://www.reddit.com${post.permalink}`}
          onClick={(e) => {
            e.preventDefault();
            openUrl(`https://www.reddit.com${post.permalink}`).catch(() => {});
          }}
        >
          <span className="news-title">{post.title}</span>
          <span className="news-meta">
            <span className="news-author">u/{post.author}</span>
            <span className="news-separator">·</span>
            <span className="news-score">▲ {post.score}</span>
            <span className="news-separator">·</span>
            <span className="news-comments">{post.num_comments} comments</span>
            <span className="news-separator">·</span>
            <span className="news-time">{relativeTime(post.created_utc)}</span>
          </span>
        </a>
      ))}
    </div>
  );
}
