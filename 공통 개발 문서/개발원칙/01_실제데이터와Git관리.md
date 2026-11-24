# 실제 데이터와 Git 관리 원칙

실제 학습 자료와 사용자 기록은 Git에 올리지 않는다. Android와 Window 모두 같은 원칙을 적용한다. 저장소에는 코드·설계·스키마와 실제 자료를 포함하지 않는 검증 도구를 관리한다.

## 보관 위치와 제외 범위

| 대상 | 보관과 Git 처리 |
| --- | --- |
| 실제 질문·답, 원문, 문제집, 보충 정보, 변환 결과와 채점 예시 | 루트 `data/` 아래 로컬 보관, 폴더 전체 ignore |
| 별도로 가져온 개인 자료 | `data/` 또는 `local-data/` 아래 로컬 보관, ignore |
| 앱 실행 중 응답·학습 기록·DB와 백업 | 플랫폼별 사용자 데이터 위치, 저장소 안에 생긴 DB·백업도 ignore |
| 앱 자산에 복사한 실제 자료와 빌드·배포 패키지 | `.gitignore`의 InitialData·bin·obj·build·artifacts 등으로 제외 |
| 명세·필드 설명과 직접 만든 가상 예시 | 공통 문서 또는 코드에서 관리 가능 |
| 검증 결과 | 개수·통과 여부·오류 코드만 기록. 실제 문항·정답·입력 답변을 넣지 않음 |

데이터의 주제가 Android여도 Android 앱 소스 폴더로 옮기지 않는다. 학습 자료는 플랫폼과 독립된 로컬 자료다. 파일 확장자만으로 전체 JSON을 제외하지 않고, 실제 자료의 보관 폴더를 분리한다.

## 새 작업 환경의 자료 준비

새로 clone한 저장소에는 실제 `data/`가 없다. 사용자가 별도로 보관한 자료를 로컬에 준비한다. 현재 Windows의 초기 자료 경로는 다음과 같다.

```text
data/initial/
  catalog.json
  android/learning.json
  android/supplement.json
  basic-cs/learning.json
  basic-cs/supplement.json
```

초기 자료 검증은 이 로컬 파일이 있을 때 실행한다. 없으면 자료 준비가 필요함을 안내하고 실제 데이터 검증을 통과로 처리하지 않는다. 개발 문서의 로컬 자료 참조 링크는 새 clone에서 대상 파일이 없을 수 있다.

Windows 빌드는 로컬 자료를 출력·publish 폴더에 복사할 수 있지만, 복사본과 그 자료를 포함한 패키지도 Git에 커밋하지 않는다. Android의 자료 배치 경로를 정할 때도 해당 복사본을 ignore에 추가한다.

## 커밋 전 확인

`.gitignore`를 실제 자료보다 먼저 적용하고 `git check-ignore`와 `git status`로 제외 여부를 확인한다. `git ls-files -- data`에 파일이 나타나면 이미 추적 중인 자료이므로 로컬 원본을 보존하며 추적을 해제한 뒤 진행한다. 실제 자료를 `git add -f`로 강제 추가하거나 문서·테스트 fixture에 복사해 우회하지 않는다.

이번 정리에서 기존 추적 파일은 루트 README뿐이었고 data의 추적 파일과 가져온 Git 이력의 data 변경은 없었다. 원본 자료를 삭제하거나 Git 이력을 변경하지 않고 ignore를 적용한다.

제외 패턴은 루트 [.gitignore](../../.gitignore), Windows의 자료 연결은 [초기 자료 연결](../../Window/구현설계/03_초기자료연결.md)을 따른다.
