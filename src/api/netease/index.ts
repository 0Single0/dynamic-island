import { axiosInstance, request } from "@/utils/request";
import type { ApiResponse } from "@/types/api";

/**
 * 网易云音乐相关接口（占位）。
 * BASE_URL 指向本地 NeteaseCloudMusicApi 服务，接入时启动即可。
 */
const BASE_URL = "http://localhost:3000";

export interface NeteaseSong {
  id: number;
  name: string;
  artists: string[];
  album?: string;
  duration?: number;
}

export interface NeteasePlayerState {
  playing: boolean;
  progress: number;
  song: NeteaseSong | null;
}

export function getPlayerState() {
  return request<NeteasePlayerState>({
    url: `${BASE_URL}/player/state`,
    method: "GET",
  });
}

export function searchSongs(keyword: string) {
  return request<{ songs: NeteaseSong[]; total: number }>({
    url: `${BASE_URL}/search`,
    method: "GET",
    params: { keywords: keyword },
  });
}

/** 原始实例调用示例：非信封结构的第三方接口可用 axiosInstance 直接请求 */
export async function getLyricRaw(songId: number): Promise<string> {
  const response = await axiosInstance.get<ApiResponse<{ lyric: string }>>(
    `${BASE_URL}/lyric`,
    { params: { id: songId } },
  );
  return response.data.data.lyric;
}
