import {Injectable} from '@angular/core';
import {Tab} from "../../models/tab";
import {Session} from "../../models/session";
import {BehaviorSubject, Observable, Subject} from "rxjs";
import {Guid} from "guid-typescript";

@Injectable({
  providedIn: 'root'
})
export class SessionService {
  sessionCounter = 0;
  currentSession$: Observable<Session>;
  sessions: Session[] = new Array<Session>();
  tabs$: Observable<Tab[]>

  private _currentSession: BehaviorSubject<Session> = new BehaviorSubject<Session>(new Session("New Session"));
  private _tabs: BehaviorSubject<Tab[]> = new BehaviorSubject<Tab[]>([]);

  constructor() {
    this.tabs$ = this._tabs.asObservable();
    this.currentSession$ = this._currentSession.asObservable();
  }

  public getSessionsTabs() {
    const tabs = this.sessions.map(session => new Tab(session.id, session.name))
    this._tabs.next(tabs);
  }

  public setCurrentSession(session: Session) {
    this._currentSession.next(session);
    this.getSessionsTabs();
  }

  public setCurrentSessionById(id: Guid) {
    const session = this.sessions.find(session => session.id.equals(id));
    if (session) {
      this.setCurrentSession(session);
    }
  }

  public addSession(name: string) {
    this.sessionCounter++;
    const newSession = new Session(name);
    this.sessions.push(newSession);
    if (this.sessionCounter == 1) {
      this.setCurrentSession(newSession);
    }
  }

  public closeSession(id: Guid) {
    this.sessionCounter--;
    this.sessions = this.sessions.filter(session => !session.id.equals(id));
    if (this.sessionCounter == 1) {
      this.setCurrentSession(this.sessions[0]);
    }
  }
}
