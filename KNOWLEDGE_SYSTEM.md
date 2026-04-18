# Graphify Knowledge System

이 프로젝트는 graphify로 지식 그래프를 관리합니다. AI 에이전트는 작업 중 발견한 지식을 `graphify-out/` 디렉토리에 축적합니다.

## 세션 시작 시 반드시 수행

1. `graphify-out/_KNOWLEDGE_*.md` 파일들을 읽어서 이전 세션에서 축적된 지식을 파악하세요
2. `graphify-out/GRAPH_REPORT.md`를 읽어서 프로젝트 구조(god nodes, communities, knowledge gaps)를 확인하세요
3. 이전 지식을 기반으로 현재 작업의 컨텍스트를 파악하세요

## 작업 중 지식 저장

코드를 분석하거나 작업하면서 다음을 발견하면 **즉시** `graphify-out/` 에 `.md` 파일로 저장하세요:

### 파일 생성 규칙

파일명: `graphify-out/_KNOWLEDGE_<제목>.md`

```markdown
---
type: knowledge
knowledge_type: <pattern|decision|convention|coupling|preference|bug_pattern|domain>
confidence: 0.50
observations: 1
first_seen: <unix_timestamp>
last_seen: <unix_timestamp>
tags: [태그1, 태그2]
related_nodes: [node_id1, node_id2]
---

# 제목

설명. 왜 이것이 중요한지, 어떤 맥락에서 발견했는지 기록.

## Related

[[node_id1]] [[node_id2]]
```

### 기존 지식 강화

같은 패턴을 다시 발견하면 **새 파일을 만들지 말고** 기존 `_KNOWLEDGE_*.md` 파일을 수정하세요:
- `observations` 값을 1 증가
- `confidence`를 `1.0 - (1.0 - 현재값) * 0.8`로 계산 (반복 관찰 시 점근적 증가)
- `last_seen`을 현재 timestamp로 갱신
- 새로운 설명을 `---` 구분자로 추가
- `related_nodes`에 새로 발견된 노드 추가

### 저장할 지식 유형

| Type | 언제 | 예시 |
|------|------|------|
| `pattern` | 아키텍처 패턴 발견 시 | "Repository 패턴으로 DB 접근" |
| `decision` | 설계 결정의 이유를 파악했을 때 | "JWT 선택: 마이크로서비스 stateless 필요" |
| `convention` | 코드 컨벤션 발견 시 | "에러는 항상 AppError로 래핑" |
| `coupling` | 모듈 간 결합 관계 발견 시 | "auth 변경 시 session도 변경 필요" |
| `preference` | 사용자 선호를 파악했을 때 | "함수형 스타일 선호, 깊은 상속 회피" |
| `bug_pattern` | 반복되는 버그 패턴 발견 시 | "pagination에서 off-by-one 에러 반복" |
| `domain` | 도메인 개념 이해 시 | "trade 상태: pending → filled → cancelled" |

## 지식 활용

작업 중 관련 지식이 있는지 확인할 때:
- `graphify-out/_KNOWLEDGE_*.md` 파일의 제목과 태그를 검색
- `graphify query "<질문>"` 으로 그래프에서 관련 노드 검색
- `graphify explain "<노드명>"` 으로 노드의 연결 관계 확인

## graphify-out/ 파일 구조

| 파일 | 역할 |
|------|------|
| `_KNOWLEDGE_*.md` | AI가 축적한 학습 지식 (핵심) |
| `_INSIGHT_*.md` | AI가 발견한 인사이트 (패턴 간 관계) |
| `_NOTE_*.md` | AI가 작성한 자유 메모 |
| `_COMMUNITY_*.md` | 커뮤니티 개요 (자동 생성) |
| `*.md` | 코드 노드별 노트 (자동 생성) |
| `GRAPH_REPORT.md` | 분석 리포트 (자동 생성) |
| `graph.json` | 지식 그래프 캐시 (vault에서 자동 생성) |
| `graph.html` | 인터랙티브 시각화 |
